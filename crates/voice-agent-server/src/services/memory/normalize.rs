//! Text normalization: `normalize` for the index, queries and tombstone fingerprints, `compare_key`
//! for telling whether two bodies say the same thing.

use unicode_normalization::UnicodeNormalization;

/// `fold_vietnamese` only knows precomposed characters, so NFC has to come first: text pasted from
/// macOS may be NFD.
pub(super) fn normalize(text: &str) -> String {
    text.nfc()
        .map(llm_wiki::fold_vietnamese)
        .flat_map(char::to_lowercase)
        .collect()
}

/// NFC and case only. "bàn nhà" and "bán nhà" are different facts, so diacritics stay, unlike in
/// `normalize`, where losing them is what lets ASR text without diacritics find a fact.
pub(super) fn compare_key(text: &str) -> String {
    text.nfc().flat_map(char::to_lowercase).collect()
}

#[cfg(test)]
mod tests {
    use super::{compare_key, normalize};

    #[test]
    fn memory_compare_key_folds_case_and_unicode_form_but_keeps_diacritics() {
        assert_eq!(compare_key("Đa\u{0323}\u{0306}t LỊCH"), "đặt lịch");
        assert_ne!(compare_key("bàn nhà"), compare_key("bán nhà"));
        assert_ne!(compare_key("bàn nhà"), compare_key("ban nha"));
    }

    #[test]
    fn memory_normalize_folds_vietnamese_both_ways() {
        for (accented, plain) in [
            ("đặt", "dat"),
            ("được", "duoc"),
            ("nhắc", "nhac"),
            ("việc", "viec"),
            ("Đặt Lịch", "dat lich"),
        ] {
            assert_eq!(normalize(accented), plain, "{accented}");
            assert_eq!(normalize(plain), plain, "{plain}");
        }
    }

    #[test]
    fn memory_normalize_treats_nfd_like_nfc() {
        // "đặt lịch" with every mark as a separate combining code point.
        let nfd = "đa\u{0323}\u{0306}t li\u{0323}ch";
        assert_eq!(normalize(nfd), "dat lich");
    }
}
