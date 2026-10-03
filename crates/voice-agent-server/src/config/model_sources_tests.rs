use super::{AppConfig, validate_deployment};

fn config() -> AppConfig {
    toml::from_str(
        r#"
[server]
bind = "127.0.0.1:0"
public_ws_url = "ws://127.0.0.1:0/voice/v1/"
[provider_defaults]
vad = "test"
asr = "test"
llm = "test"
tts = "test"
"#,
    )
    .unwrap()
}

#[test]
fn prepared_sources_default_to_empty_and_accept_http_mirrors() {
    let mut config = config();
    assert!(config.deployment.models.sources.is_empty());
    assert!(validate_deployment(&config).is_ok());
    config.deployment.models.sources.insert(
        "prepared://deployment/voice.bin".into(),
        "http://127.0.0.1:8080/voice.bin".into(),
    );
    assert!(validate_deployment(&config).is_ok());
}

#[test]
fn invalid_prepared_sources_are_rejected_before_download() {
    for (source, remote) in [
        ("https://original.example/model", "https://mirror.example/model"),
        ("prepared://deployment/voice.bin", "file:///tmp/voice.bin"),
        ("prepared://deployment/voice.bin", "https://user:secret@example.com/model"),
        ("prepared://deployment/voice.bin", "https://example.com/model#fragment"),
    ] {
        let mut config = config();
        config.deployment.models.sources.insert(source.into(), remote.into());
        assert!(validate_deployment(&config).is_err());
    }
}
