use super::*;
#[test]
fn references_preserve_source_and_reject_malformed_or_wrong_kind() {
    let db = ProviderRef::parse("primary", Some("llm")).unwrap();
    let config = ProviderRef::parse("config:llm:primary", Some("llm")).unwrap();
    assert_ne!(db, config);
    assert_eq!(config.key, "primary");
    assert_eq!(config.public_key(), "config:llm:primary");
    assert_eq!(db.public_key(), "primary");
    for key in [
        "config:tts:primary",
        "config:llm:",
        "config:llm:a:b",
        "config:vision:a",
        "config:llm:../a",
        "config:llm:a b",
        "config:llm",
    ] {
        assert!(ProviderRef::parse(key, Some("llm")).is_err(), "{key}");
    }
}
