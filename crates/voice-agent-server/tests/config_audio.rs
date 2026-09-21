use std::net::SocketAddr;

use url::Url;
use voice_agent_server::config::{
    AppConfig, AudioConfig, AuthConfig, LimitsConfig, ServerConfig, WebsocketConfig,
};

fn valid_config() -> AppConfig {
    AppConfig {
        server: ServerConfig {
            bind: "127.0.0.1:8000".parse::<SocketAddr>().unwrap(),
            public_ws_url: Url::parse("ws://127.0.0.1:8000/voice/v1/").unwrap(),
            hello_timeout_ms: 5_000,
        },
        auth: AuthConfig::default(),
        audio: AudioConfig::default(),
        websocket: WebsocketConfig::default(),
        limits: LimitsConfig::default(),
    }
}

#[test]
fn audio_limit_is_converted_to_an_exact_frame_capacity() {
    let config = valid_config();
    assert_eq!(config.max_capture_frames(), 500);
    assert!(config.validate().is_ok());
}

#[test]
fn invalid_audio_and_transport_limits_fail_fast() {
    let mut config = valid_config();
    config.audio.max_utterance_ms = 1_000;
    assert!(config.validate().is_err());

    let mut config = valid_config();
    config.websocket.max_frame_bytes = 3_999;
    assert!(config.validate().is_err());
}
