use voice_agent_server::config::AppConfig;

#[test]
fn vision_config_uses_llm_provider_instances_with_vision_tag_and_redacts_secret() {
    let config: AppConfig = toml::from_str(
        r#"
[server]
bind = "127.0.0.1:8000"
public_ws_url = "ws://127.0.0.1:8000/voice/v1/"
[provider_defaults]
vad="v"
asr="a"
llm="one"
tts="t"
vision="two"
[vision]
enabled = true
[providers.vad.instances.v]
adapter="silero_onnx"
[providers.asr.instances.a]
adapter="zipformer_sherpa"
[providers.tts.instances.t]
adapter="edge_tts"
[providers.llm.instances.one]
adapter="openai"
base_url="http://127.0.0.1:3001/v1/"
api_key="secret-a"
model="model-a"
tags=["chat"]
[providers.llm.instances.two]
adapter="openai"
base_url="http://127.0.0.1:3002/v1/"
api_key="secret-b"
model="model-b"
tags=["chat", "vision"]
"#,
    )
    .unwrap();
    assert_eq!(config.providers.llm.instances.len(), 2);
    assert_eq!(config.providers.llm.instances["two"].openai().model, "model-b");
    assert!(config.validate().is_ok());
    assert!(!format!("{config:?}").contains("secret-a"));
}

#[test]
fn vision_config_rejects_bound_llm_instance_without_vision_tag() {
    let config: AppConfig = toml::from_str(
        r#"
[server]
bind = "127.0.0.1:8000"
public_ws_url = "ws://127.0.0.1:8000/voice/v1/"
[provider_defaults]
vad="v"
asr="a"
llm="one"
tts="t"
vision="one"
[vision]
enabled = true
[providers.vad.instances.v]
adapter="silero_onnx"
[providers.asr.instances.a]
adapter="zipformer_sherpa"
[providers.tts.instances.t]
adapter="edge_tts"
[providers.llm.instances.one]
adapter="openai"
base_url="http://127.0.0.1:3001/v1/"
api_key="secret-a"
model="model-a"
tags=["chat"]
"#,
    )
    .unwrap();
    assert!(config.validate().is_err());
}
