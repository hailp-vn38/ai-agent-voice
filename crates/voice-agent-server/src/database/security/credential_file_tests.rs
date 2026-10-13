use super::*;

#[test]
fn key_file_credentials_survive_reload_and_rotation() {
    let path = std::env::temp_dir().join(format!("credential-{}.json", uuid::Uuid::new_v4()));
    let write = |version, keys| {
        std::fs::write(
            &path,
            serde_json::to_vec(&serde_json::json!({
                "current_version": version, "keys": keys
            }))
            .unwrap(),
        )
        .unwrap();
    };
    write(1, serde_json::json!({"1": BASE64.encode([7; 32])}));
    let record = CredentialCipher::from_file(&path)
        .unwrap()
        .seal(&SecretValue::new("test-credential".into()), "mcp:gateway")
        .unwrap();
    let reference =
        SecretRef::parse(reference("mcp:gateway", Some(&record), None).unwrap()).unwrap();
    write(
        2,
        serde_json::json!({"1": BASE64.encode([7; 32]), "2": BASE64.encode([8; 32])}),
    );
    let cipher = CredentialCipher::from_file(&path).unwrap();
    assert_eq!(
        cipher.resolve(&reference).unwrap().expose(),
        "test-credential"
    );
    let new_record = cipher
        .seal(&SecretValue::new("replacement".into()), "mcp:gateway")
        .unwrap();
    assert_eq!(metadata(Some(&new_record))["key_version"], 2);
    for keys in [
        serde_json::json!({}),
        serde_json::json!({"1": "invalid"}),
        serde_json::json!({"1": BASE64.encode([7; 31])}),
        serde_json::json!({"0": BASE64.encode([7; 32])}),
    ] {
        write(1, keys);
        assert!(CredentialCipher::from_file(&path).is_err());
    }
    std::fs::remove_file(&path).unwrap();
    assert!(CredentialCipher::from_file(&path).is_err());
}
