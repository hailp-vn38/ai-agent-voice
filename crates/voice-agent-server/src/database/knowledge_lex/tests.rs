use super::*;

#[test]
fn terms_are_safe_with_a_loose_fallback() {
    let c = compile("how does Agent Session recover?").unwrap();
    assert_eq!(c.strict, "\"how\"* AND \"does\"* AND \"Agent\"* AND \"Session\"* AND \"recover\"*");
    assert_eq!(c.broad.unwrap(), "\"how\"* OR \"does\"* OR \"Agent\"* OR \"Session\"* OR \"recover\"*");
}

#[test]
fn compound_identifiers_and_quotes_have_literal_semantics() {
    let c = compile("DEC-0054 gpt-4 \"Agent Session\"").unwrap();
    assert_eq!(c.strict, "\"DEC 0054\" AND \"gpt 4\" AND \"Agent Session\"");
    assert!(!c.strict.contains(" NOT "));
    assert!(!c.strict.contains(':'));
    assert!(compile("abc\"").is_err());
}

#[test]
fn vietnamese_and_control_syntax_cannot_inject_fts() {
    let c = compile("tìm kiếm thông tin").unwrap();
    assert_eq!(c.strict, "\"tìm\"* AND \"kiếm\"* AND \"thông\"* AND \"tin\"*");
    let injected = compile("auth OR title:secret").unwrap();
    assert!(!injected.strict.contains("title:"));
    assert!(compile("!@#$%^").is_err());
    assert!(compile(&"x".repeat(257)).is_err());
    assert!(compile(&"abc ".repeat(22)).is_err());
}
