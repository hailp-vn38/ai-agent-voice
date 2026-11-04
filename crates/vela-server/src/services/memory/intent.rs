//! Intent gate: which Memory writes the committed User text of this turn explicitly asked for.

use super::types::ExplicitIntent;
use unicode_normalization::UnicodeNormalization;

const REMEMBER: &[&str] = &[
    "nhớ giúp",
    "ghi nhớ",
    "lưu lại",
    "nhớ rằng",
    "đừng quên",
    "nho giup",
    "ghi nho",
    "luu lai",
    "remember",
    "don't forget",
];
const CORRECT: &[&str] = &[
    "sửa lại",
    "đính chính",
    "nhớ nhầm",
    "cập nhật lại",
    "sua lai",
    "dinh chinh",
    "correct that",
    "update that",
];
const FORGET: &[&str] = &[
    "quên",
    "xóa ký ức",
    "xoá ký ức",
    "quen di",
    "xoa ky uc",
    "forget",
];
/// These mean "remember", yet contain a forget phrase.
const NOT_FORGET: &[&str] = &["đừng quên", "don't forget"];

/// Matches on NFC plus lowercase and keeps diacritics: folding everything would turn `quên` into
/// `quen`, so "tôi quen anh ấy" would unlock Forget.
pub fn classify_intent(committed_user_text: &str) -> ExplicitIntent {
    let text: String = committed_user_text
        .nfc()
        .flat_map(char::to_lowercase)
        .map(|c| if c == '\u{2019}' { '\'' } else { c })
        .collect();
    let forget_text = NOT_FORGET
        .iter()
        .fold(text.clone(), |text, phrase| text.replace(phrase, " "));
    ExplicitIntent {
        remember: contains_any(&text, REMEMBER),
        correct: contains_any(&text, CORRECT),
        forget: contains_any(&forget_text, FORGET),
    }
}

fn contains_any(text: &str, phrases: &[&str]) -> bool {
    phrases.iter().any(|phrase| contains_phrase(text, phrase))
}

/// Whole-word match, so `quen di` does not fire inside "quen dinh".
fn contains_phrase(text: &str, phrase: &str) -> bool {
    text.match_indices(phrase).any(|(start, _)| {
        let before = text[..start].chars().next_back();
        let after = text[start + phrase.len()..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
}

#[cfg(test)]
mod tests {
    use super::classify_intent;

    fn flags(text: &str) -> (bool, bool, bool) {
        let intent = classify_intent(text);
        (intent.remember, intent.correct, intent.forget)
    }

    #[test]
    fn memory_intent_covers_the_remember_row_of_the_gate_table() {
        for text in [
            "nhớ giúp tôi mua sữa",
            "ghi nhớ điều này",
            "lưu lại số này",
            "nhớ rằng tôi dị ứng tôm",
            "đừng quên mua sữa",
            "nho giup toi",
            "ghi nho nhe",
            "luu lai giup",
            "remember my birthday",
            "don't forget the milk",
            "Don\u{2019}t forget the milk",
        ] {
            assert_eq!(flags(text), (true, false, false), "{text}");
        }
    }

    #[test]
    fn memory_intent_covers_the_correct_row_of_the_gate_table() {
        for text in [
            "sửa lại giúp tôi",
            "đính chính một chút",
            "bạn nhớ nhầm rồi",
            "cập nhật lại đi",
            "sua lai nhe",
            "dinh chinh nhe",
            "correct that please",
            "update that for me",
        ] {
            assert_eq!(flags(text), (false, true, false), "{text}");
        }
    }

    #[test]
    fn memory_intent_covers_the_forget_row_of_the_gate_table() {
        for text in [
            "quên chuyện đó đi",
            "xóa ký ức đó",
            "xoá ký ức đó",
            "quen di nhe",
            "xoa ky uc nay",
            "forget that",
            "FORGET it",
        ] {
            assert_eq!(flags(text), (false, false, true), "{text}");
        }
    }

    #[test]
    fn memory_intent_keeps_diacritics_so_ordinary_speech_does_not_unlock_forget() {
        assert_eq!(flags("tôi quen anh ấy"), (false, false, false));
        assert_eq!(flags("quen dinh roi"), (false, false, false));
        assert_eq!(flags("hôm nay trời đẹp"), (false, false, false));
    }

    #[test]
    fn memory_intent_is_per_action() {
        assert_eq!(flags("nhớ giúp tôi số này"), (true, false, false));
        assert_eq!(flags("đừng quên mua sữa"), (true, false, false));
        assert_eq!(flags("don't forget the milk"), (true, false, false));
        assert_eq!(
            flags("đừng quên mua sữa, và quên chuyện kia đi"),
            (true, false, true)
        );
    }

    #[test]
    fn memory_intent_matches_nfd_input_like_nfc() {
        assert_eq!(
            flags("que\u{0302}n chuye\u{0323}\u{0302}n đó"),
            (false, false, true)
        );
    }
}
