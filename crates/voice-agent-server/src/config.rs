use serde::Deserialize;
use std::{fs, net::SocketAddr, path::Path};
use thiserror::Error;
use url::Url;

#[derive(Clone, Debug, Deserialize)]
pub struct AppConfig {
    pub server: ServerConfig,
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default)]
    pub audio: AudioConfig,
    #[serde(default)]
    pub websocket: WebsocketConfig,
    #[serde(default)]
    pub limits: LimitsConfig,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ServerConfig {
    pub bind: SocketAddr,
    pub public_ws_url: Url,
    #[serde(default = "default_hello_timeout_ms")]
    pub hello_timeout_ms: u64,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct AuthConfig {
    #[serde(default)]
    pub token: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct AudioConfig {
    #[serde(default = "default_input_rate")]
    pub input_sample_rate: u32,
    #[serde(default = "default_output_rate")]
    pub output_sample_rate: u32,
    #[serde(default = "default_channels")]
    pub channels: u8,
    #[serde(default = "default_frame_ms")]
    pub frame_ms: u16,
    #[serde(default = "default_max_utterance_ms")]
    pub max_utterance_ms: u64,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            input_sample_rate: default_input_rate(),
            output_sample_rate: default_output_rate(),
            channels: default_channels(),
            frame_ms: default_frame_ms(),
            max_utterance_ms: default_max_utterance_ms(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct WebsocketConfig {
    #[serde(default = "default_max_frame_bytes")]
    pub max_frame_bytes: usize,
}

impl Default for WebsocketConfig {
    fn default() -> Self {
        Self {
            max_frame_bytes: default_max_frame_bytes(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct LimitsConfig {
    #[serde(default = "default_queue_capacity")]
    pub session_event_queue: usize,
    #[serde(default = "default_queue_capacity")]
    pub outbound_control_queue: usize,
    #[serde(default = "default_queue_capacity")]
    pub outbound_audio_queue: usize,
}

impl Default for LimitsConfig {
    fn default() -> Self {
        Self {
            session_event_queue: default_queue_capacity(),
            outbound_control_queue: default_queue_capacity(),
            outbound_audio_queue: default_queue_capacity(),
        }
    }
}

fn default_hello_timeout_ms() -> u64 {
    5_000
}
fn default_input_rate() -> u32 {
    16_000
}
fn default_output_rate() -> u32 {
    24_000
}
fn default_channels() -> u8 {
    1
}
fn default_frame_ms() -> u16 {
    60
}
fn default_max_frame_bytes() -> usize {
    65_536
}
fn default_max_utterance_ms() -> u64 {
    30_000
}
fn default_queue_capacity() -> usize {
    32
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("cannot read config: {0}")]
    Read(#[from] std::io::Error),
    #[error("invalid TOML: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("invalid configuration: {0}")]
    Validation(String),
}

impl AppConfig {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let config: Self = toml::from_str(&fs::read_to_string(path)?)?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.server.public_ws_url.scheme() != "ws" && self.server.public_ws_url.scheme() != "wss"
        {
            return Err(ConfigError::Validation(
                "server.public_ws_url must use ws or wss".into(),
            ));
        }
        if self.server.hello_timeout_ms == 0
            || !(4_000..=1_048_576).contains(&self.websocket.max_frame_bytes)
        {
            return Err(ConfigError::Validation(
                "hello timeout and WebSocket maximum frame size must be valid".into(),
            ));
        }
        if self.audio.input_sample_rate != 16_000
            || self.audio.output_sample_rate != 24_000
            || self.audio.channels != 1
            || self.audio.frame_ms != 60
        {
            return Err(ConfigError::Validation(
                "V1 requires canonical audio: uplink 16 kHz, downlink 24 kHz, mono, 60 ms".into(),
            ));
        }
        if !(1_000..=120_000).contains(&self.audio.max_utterance_ms)
            || !self
                .audio
                .max_utterance_ms
                .is_multiple_of(u64::from(self.audio.frame_ms))
        {
            return Err(ConfigError::Validation(
                "audio.max_utterance_ms must be 1000..=120000 and divisible by frame_ms".into(),
            ));
        }
        if [
            self.limits.session_event_queue,
            self.limits.outbound_control_queue,
            self.limits.outbound_audio_queue,
        ]
        .contains(&0)
        {
            return Err(ConfigError::Validation(
                "queue capacities must be positive".into(),
            ));
        }
        Ok(())
    }

    pub fn max_capture_frames(&self) -> usize {
        (self.audio.max_utterance_ms / u64::from(self.audio.frame_ms)) as usize
    }
}
