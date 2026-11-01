//! NFC plus Vietnamese folding, shared by the index, queries and tombstone fingerprints.

use unicode_normalization::UnicodeNormalization;

/// `fold_vietnamese` only knows precomposed characters, so NFC has to come first: text pasted from
/// macOS may be NFD.
pub(super) fn normalize(text: &str) -> String {
    text.nfc()
        .map(llm_wiki::fold_vietnamese)
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::normalize;

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
