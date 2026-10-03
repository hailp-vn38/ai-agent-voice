use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use url::Url;

use super::defaults::*;
pub use crate::providers::{
    asr::{
        gipformer::config::GipformerSherpaOfflineConfig, zipformer::config::ZipformerSherpaConfig,
    },
    llm::openai::config::OpenAiConfig,
    tts::{
        chillaudio::config::ChillAudioWsConfig,
        kokoro_vi::config::KokoroViOnnxConfig,
        zerotts::config::{ZeroTtsDeliveryMode, ZeroTtsOnnxConfig},
    },
};

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProvidersConfig {
    #[serde(default)]
    pub vad: VadProvidersConfig,
    #[serde(default)]
    pub asr: AsrProvidersConfig,
    #[serde(default)]
    pub llm: LlmProvidersConfig,
    #[serde(default)]
    pub tts: TtsProvidersConfig,
    #[serde(default)]
    pub vision: VisionProvidersConfig,
}

macro_rules! provider_catalog_config {
    ($name:ident, $instance:ident) => {
        #[derive(Clone, Debug, Default, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub struct $name {
            #[serde(default)]
            pub instances: BTreeMap<String, $instance>,
        }
    };
}

provider_catalog_config!(VadProvidersConfig, VadInstanceConfig);
provider_catalog_config!(AsrProvidersConfig, AsrInstanceConfig);
provider_catalog_config!(LlmProvidersConfig, LlmInstanceConfig);
provider_catalog_config!(TtsProvidersConfig, TtsInstanceConfig);
provider_catalog_config!(VisionProvidersConfig, VisionInstanceConfig);

#[derive(Clone, Default, Deserialize)]
pub struct SecretString(pub(crate) String);

#[allow(dead_code)]
impl SecretString {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for SecretString {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "adapter", rename_all = "snake_case")]
pub enum LlmInstanceConfig {
    Openai(OpenAiConfig),
}

impl LlmInstanceConfig {
    pub const fn adapter(&self) -> &'static str {
        match self {
            Self::Openai(_) => "openai",
        }
    }
    pub fn openai(&self) -> &OpenAiConfig {
        match self {
            Self::Openai(config) => config,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "adapter", rename_all = "snake_case")]
pub enum VisionInstanceConfig {
    #[serde(rename = "openai_vision")]
    OpenAiVision(OpenAiVisionConfig),
}

impl VisionInstanceConfig {
    pub const fn adapter(&self) -> &'static str {
        "openai_vision"
    }
    pub fn openai_vision(&self) -> &OpenAiVisionConfig {
        match self {
            Self::OpenAiVision(config) => config,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenAiVisionConfig {
    pub base_url: Url,
    #[serde(default)]
    pub api_key: SecretString,
    pub model: String,
    #[serde(default = "default_vision_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default = "default_vision_max_tokens")]
    pub max_tokens: u32,
    #[serde(default = "default_vision_temperature")]
    pub temperature: f32,
    #[serde(default = "default_vision_top_p")]
    pub top_p: f32,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "adapter", rename_all = "snake_case")]
pub enum TtsInstanceConfig {
    #[serde(rename = "zerotts_onnx")]
    ZeroTtsOnnx(ZeroTtsOnnxConfig),
    #[serde(rename = "chillaudio_ws")]
    ChillAudioWs(ChillAudioWsConfig),
    #[serde(rename = "kokoro_vi_onnx")]
    KokoroViOnnx(KokoroViOnnxConfig),
}

impl TtsInstanceConfig {
    pub const fn adapter(&self) -> &'static str {
        match self {
            Self::ZeroTtsOnnx(_) => "zerotts_onnx",
            Self::ChillAudioWs(_) => "chillaudio_ws",
            Self::KokoroViOnnx(_) => "kokoro_vi_onnx",
        }
    }
    pub const fn preload(&self) -> bool {
        match self {
            Self::ZeroTtsOnnx(config) => config.preload,
            Self::ChillAudioWs(config) => config.preload,
            Self::KokoroViOnnx(config) => config.preload,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "adapter", rename_all = "snake_case")]
pub enum VadInstanceConfig {
    #[serde(rename = "silero_onnx")]
    SileroOnnx(SileroOnnxConfig),
}

impl VadInstanceConfig {
    pub const fn adapter(&self) -> &'static str {
        "silero_onnx"
    }
    pub fn silero_onnx(&self) -> &SileroOnnxConfig {
        match self {
            Self::SileroOnnx(config) => config,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SileroOnnxConfig {
    #[serde(default = "default_vad_model")]
    pub model: String,
    #[serde(default = "default_provider_threads")]
    pub num_threads: i32,
    #[serde(default = "default_min_speech_ms")]
    pub min_speech_ms: u64,
    #[serde(default = "default_end_silence_ms")]
    pub end_silence_ms: u64,
    #[serde(default = "default_pre_roll_ms")]
    pub pre_roll_ms: u64,
    #[serde(default = "default_speech_threshold")]
    pub speech_threshold: f32,
    #[serde(default = "default_exit_threshold")]
    pub exit_threshold: f32,
}

impl Default for SileroOnnxConfig {
    fn default() -> Self {
        Self {
            model: default_vad_model(),
            num_threads: default_provider_threads(),
            min_speech_ms: default_min_speech_ms(),
            end_silence_ms: default_end_silence_ms(),
            pre_roll_ms: default_pre_roll_ms(),
            speech_threshold: default_speech_threshold(),
            exit_threshold: default_exit_threshold(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "adapter", rename_all = "snake_case")]
pub enum AsrInstanceConfig {
    #[serde(rename = "zipformer_sherpa")]
    ZipformerSherpa(ZipformerSherpaConfig),
    GipformerSherpaOffline(GipformerSherpaOfflineConfig),
}

impl AsrInstanceConfig {
    pub const fn adapter(&self) -> &'static str {
        match self {
            Self::ZipformerSherpa(_) => "zipformer_sherpa",
            Self::GipformerSherpaOffline(_) => "gipformer_sherpa_offline",
        }
    }
    pub fn model(&self) -> &str {
        match self {
            Self::ZipformerSherpa(config) => &config.model,
            Self::GipformerSherpaOffline(config) => &config.model,
        }
    }
}

#[cfg(test)]
mod zerotts_delivery_tests {
    use super::{ZeroTtsDeliveryMode, ZeroTtsOnnxConfig};
    #[test]
    fn stream_delivery_is_default_and_file_remains_selectable() {
        let default: ZeroTtsOnnxConfig = toml::from_str("").unwrap();
        assert_eq!(default.delivery_mode, ZeroTtsDeliveryMode::Stream);
        assert_eq!(
            ZeroTtsOnnxConfig::default().delivery_mode,
            ZeroTtsDeliveryMode::Stream
        );
        let selected: ZeroTtsOnnxConfig = toml::from_str("delivery_mode = 'file'").unwrap();
        assert_eq!(selected.delivery_mode, ZeroTtsDeliveryMode::File);
    }
}
