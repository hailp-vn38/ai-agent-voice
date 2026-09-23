use super::*;

#[derive(Clone, Debug, Default, Deserialize)]
pub struct ProvidersConfig {
    pub vad: VadProviderConfig,
    pub asr: AsrProviderConfig,
    #[serde(default)]
    pub llm: LlmProviderConfig,
    #[serde(default)]
    pub tts: TtsProviderConfig,
}

#[derive(Clone, Default, Deserialize)]
pub struct SecretString(String);

#[allow(dead_code)]
impl SecretString {
    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for SecretString {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LlmProviderConfig {
    #[serde(rename = "type", default = "default_llm_adapter")]
    pub adapter: String,
    #[serde(default)]
    pub openai: Option<OpenAiConfig>,
}

impl Default for LlmProviderConfig {
    fn default() -> Self {
        Self {
            adapter: default_llm_adapter(),
            openai: Some(OpenAiConfig::default()),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenAiConfig {
    #[serde(default)]
    pub api_key: SecretString,
    #[serde(default = "default_openai_base_url")]
    pub base_url: Url,
    #[serde(default = "default_openai_model")]
    pub model: String,
    #[serde(default = "default_llm_timeout_ms")]
    pub timeout_ms: u64,
}

impl Default for OpenAiConfig {
    fn default() -> Self {
        Self {
            api_key: SecretString(String::new()),
            base_url: default_openai_base_url(),
            model: default_openai_model(),
            timeout_ms: default_llm_timeout_ms(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TtsProviderConfig {
    #[serde(default = "default_tts_adapter")]
    pub adapter: String,
    #[serde(default)]
    pub zerotts_onnx: Option<ZeroTtsOnnxConfig>,
}

impl Default for TtsProviderConfig {
    fn default() -> Self {
        Self {
            adapter: default_tts_adapter(),
            zerotts_onnx: Some(ZeroTtsOnnxConfig::default()),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ZeroTtsOnnxConfig {
    #[serde(default = "default_tts_model")]
    pub model: String,
    #[serde(default = "default_asr_threads")]
    pub num_threads: i32,
    #[serde(default = "default_tts_voice")]
    pub voice: String,
    #[serde(default)]
    pub delivery_mode: ZeroTtsDeliveryMode,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ZeroTtsDeliveryMode {
    File,
    #[default]
    Stream,
}

impl Default for ZeroTtsOnnxConfig {
    fn default() -> Self {
        Self {
            model: default_tts_model(),
            num_threads: default_asr_threads(),
            voice: default_tts_voice(),
            delivery_mode: ZeroTtsDeliveryMode::Stream,
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

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VadProviderConfig {
    #[serde(default = "default_vad_adapter")]
    pub adapter: String,
    #[serde(default)]
    pub silero_onnx: Option<SileroOnnxConfig>,
}

impl Default for VadProviderConfig {
    fn default() -> Self {
        Self {
            adapter: default_vad_adapter(),
            silero_onnx: Some(SileroOnnxConfig::default()),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
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

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AsrProviderConfig {
    #[serde(default = "default_asr_adapter")]
    pub adapter: String,
    #[serde(default)]
    pub zipformer_sherpa: Option<ZipformerSherpaConfig>,
}

impl Default for AsrProviderConfig {
    fn default() -> Self {
        Self {
            adapter: default_asr_adapter(),
            zipformer_sherpa: Some(ZipformerSherpaConfig::default()),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ZipformerSherpaConfig {
    #[serde(default = "default_asr_model")]
    pub model: String,
    #[serde(default = "default_asr_threads")]
    pub num_threads: i32,
    #[serde(default = "default_decoding_method")]
    pub decoding_method: String,
}

impl Default for ZipformerSherpaConfig {
    fn default() -> Self {
        Self {
            model: default_asr_model(),
            num_threads: default_asr_threads(),
            decoding_method: default_decoding_method(),
        }
    }
}
