//! Refuses secrets and oversized fields before they reach the vault and the prompt.

use super::types::MemoryError;
use regex::Regex;
use std::sync::LazyLock;

pub(super) const MAX_TITLE_CHARS: usize = 120;
pub(super) const MAX_HOOK_CHARS: usize = 200;

static SECRET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"\bsk-[A-Za-z0-9_-]{16,}",
        r"|\bghp_[A-Za-z0-9]{20,}",
        r"|\bxox[bap]-[A-Za-z0-9-]{10,}",
        r"|\bAKIA[0-9A-Z]{16}\b",
        r"|\beyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+",
        r"|-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----",
        r"|\b[A-Za-z][A-Za-z0-9+.-]*://[^\s/:@]+:[^\s/@]+@",
    ))
    .expect("the secret pattern is valid")
});

/// Size is checked first so an oversized value is never scanned.
pub(super) fn guard(
    title: &str,
    hook: &str,
    body: &str,
    max_body_chars: usize,
) -> Result<(), MemoryError> {
    if title.chars().count() > MAX_TITLE_CHARS
        || hook.chars().count() > MAX_HOOK_CHARS
        || body.chars().count() > max_body_chars
    {
        return Err(MemoryError::TooLarge);
    }
    if [title, hook, body].iter().any(|text| SECRET.is_match(text)) {
        return Err(MemoryError::RejectedSensitive);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(text: &str) -> Result<(), MemoryError> {
        guard("title", "hook", text, 2_000)
    }

    #[test]
    fn memory_guard_rejects_secrets_in_any_field() {
        for secret in [
            "sk-abcdefghijklmnopqrstuvwx",
            "token ghp_abcdefghijklmnopqrstuvwxyz0123456789",
            "xoxb-1234567890-abcdefghij",
            "AKIAIOSFODNN7EXAMPLE",
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.c2lnbmF0dXJl",
            "-----BEGIN RSA PRIVATE KEY-----",
            "-----BEGIN PRIVATE KEY-----",
            "postgres://admin:hunter2@db.internal/app",
            "mysql://root:pw@10.0.0.1:3306/x",
        ] {
            assert!(
                matches!(body(secret), Err(MemoryError::RejectedSensitive)),
                "{secret}"
            );
        }
        assert!(matches!(
            guard("AKIAIOSFODNN7EXAMPLE", "", "", 2_000),
            Err(MemoryError::RejectedSensitive)
        ));
        assert!(matches!(
            guard("", "AKIAIOSFODNN7EXAMPLE", "", 2_000),
            Err(MemoryError::RejectedSensitive)
        ));
    }

    #[test]
    fn memory_guard_allows_ordinary_text_about_passwords() {
        for text in [
            "Mật khẩu wifi của quán là chuyện cần nhắc, password thì đổi hàng tháng",
            "the password policy requires 12 characters",
            "task-abcdefghijklmnopqrstuvwxyz is finished",
            "see https://example.com/path?a=b",
            "email me at user@example.com",
            "ratio 10:30@home",
        ] {
            assert!(body(text).is_ok(), "{text}");
        }
    }

    #[test]
    fn memory_guard_counts_characters_not_bytes() {
        let title_at_limit = "đ".repeat(MAX_TITLE_CHARS);
        assert!(guard(&title_at_limit, "", "", 10).is_ok());
        assert!(matches!(
            guard(&format!("{title_at_limit}đ"), "", "", 10),
            Err(MemoryError::TooLarge)
        ));
        assert!(matches!(
            guard("", &"x".repeat(MAX_HOOK_CHARS + 1), "", 10),
            Err(MemoryError::TooLarge)
        ));
        assert!(guard("", "", &"ệ".repeat(10), 10).is_ok());
        assert!(matches!(
            guard("", "", &"ệ".repeat(11), 10),
            Err(MemoryError::TooLarge)
        ));
    }
}
