use voice_agent_server::config::AppConfig;

#[test]
fn vision_config_accepts_multiple_instances_and_redacts_secret() {
    let config: AppConfig = toml::from_str(
        r#"
[server]
bind = "127.0.0.1:8000"
public_ws_url = "ws://127.0.0.1:8000/voice/v1/"
[provider_defaults]
vad="v"
asr="a"
llm="l"
tts="t"
vision="one"
[providers.vision.instances.one]
adapter="openai_vision"
base_url="http://127.0.0.1:3001/v1/"
api_key="secret-a"
model="model-a"
[providers.vision.instances.two]
adapter="openai_vision"
base_url="http://127.0.0.1:3002/v1/"
api_key="secret-b"
model="model-b"
"#,
    )
    .unwrap();
    assert_eq!(config.providers.vision.instances.len(), 2);
    assert_eq!(
        config.providers.vision.instances["two"]
            .openai_vision()
            .model,
        "model-b"
    );
    assert!(!format!("{config:?}").contains("secret-a"));
}
