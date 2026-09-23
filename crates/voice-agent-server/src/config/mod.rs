use serde::Deserialize;
use std::net::SocketAddr;
use url::Url;

mod validation;

pub use validation::ConfigError;

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
    #[serde(default)]
    pub providers: ProvidersConfig,
    #[serde(default)]
    pub workers: WorkersConfig,
    #[serde(default)]
    pub deployment: DeploymentConfig,
    #[serde(default)]
    pub runtime: RuntimeConfig,
    #[serde(default)]
    pub llm: LlmConfig,
    #[serde(default)]
    pub tts: TtsConfig,
    #[serde(default)]
    pub speech_output: SpeechOutputConfig,
    #[serde(default)]
    pub barge_in: BargeInConfig,
}

mod defaults;
mod providers;

use defaults::*;
pub use providers::*;

#[derive(Clone, Debug, Default, Deserialize)]
pub struct BargeInConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub trust_client_aec_feature: bool,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct WorkersConfig {
    #[serde(default)]
    pub vad: VadWorkerConfig,
    #[serde(default)]
    pub asr: AsrWorkerConfig,
    #[serde(default)]
    pub tts: TtsWorkerConfig,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TtsWorkerConfig {
    #[serde(default = "default_asr_worker_count")]
    pub max_workers: usize,
    #[serde(default = "default_worker_queue_capacity")]
    pub command_queue_capacity: usize,
    #[serde(default = "default_cleanup_grace_ms")]
    pub cleanup_grace_ms: u64,
}

impl Default for TtsWorkerConfig {
    fn default() -> Self {
        Self {
            max_workers: default_asr_worker_count(),
            command_queue_capacity: default_worker_queue_capacity(),
            cleanup_grace_ms: default_cleanup_grace_ms(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VadWorkerConfig {
    #[serde(default = "default_vad_worker_count")]
    pub max_workers: usize,
    #[serde(default = "default_worker_queue_capacity")]
    pub command_queue_capacity: usize,
    #[serde(default = "default_vad_reset_timeout_ms")]
    pub reset_timeout_ms: u64,
    #[serde(default = "default_cleanup_grace_ms")]
    pub cleanup_grace_ms: u64,
}

impl Default for VadWorkerConfig {
    fn default() -> Self {
        Self {
            max_workers: default_vad_worker_count(),
            command_queue_capacity: default_worker_queue_capacity(),
            reset_timeout_ms: default_vad_reset_timeout_ms(),
            cleanup_grace_ms: default_cleanup_grace_ms(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AsrWorkerConfig {
    #[serde(default = "default_asr_worker_count")]
    pub max_workers: usize,
    #[serde(default = "default_worker_queue_capacity")]
    pub command_queue_capacity: usize,
    #[serde(default = "default_asr_final_timeout_ms")]
    pub final_timeout_ms: u64,
    #[serde(default = "default_cleanup_grace_ms")]
    pub cleanup_grace_ms: u64,
}

impl Default for AsrWorkerConfig {
    fn default() -> Self {
        Self {
            max_workers: default_asr_worker_count(),
            command_queue_capacity: default_worker_queue_capacity(),
            final_timeout_ms: default_asr_final_timeout_ms(),
            cleanup_grace_ms: default_cleanup_grace_ms(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeploymentConfig {
    #[serde(default = "default_manifest_path")]
    pub model_manifest: std::path::PathBuf,
    #[serde(default)]
    pub profile: String,
    #[serde(default)]
    pub model_acknowledgements: Vec<ModelAcknowledgement>,
    #[serde(default)]
    pub models: ModelStoreConfig,
}

impl Default for DeploymentConfig {
    fn default() -> Self {
        Self {
            model_manifest: default_manifest_path(),
            profile: "development-noncommercial".into(),
            model_acknowledgements: Vec::new(),
            models: ModelStoreConfig::default(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelStoreConfig {
    #[serde(default = "default_models_root")]
    pub root: std::path::PathBuf,
    #[serde(default)]
    pub offline: bool,
}

impl Default for ModelStoreConfig {
    fn default() -> Self {
        Self {
            root: default_models_root(),
            offline: false,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeConfig {
    #[serde(default)]
    pub onnx: OnnxRuntimeConfig,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OnnxRuntimeConfig {
    #[serde(default = "default_onnx_runtime_library")]
    pub library: std::path::PathBuf,
}

impl Default for OnnxRuntimeConfig {
    fn default() -> Self {
        Self {
            library: default_onnx_runtime_library(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelAcknowledgement {
    pub model: String,
    pub revision: String,
    pub license: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct LlmConfig {
    #[serde(default = "default_max_history_messages")]
    pub max_history_messages: usize,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TtsConfig {
    #[serde(default = "default_tts_timeout_ms")]
    pub timeout_ms: u64,
}

impl Default for TtsConfig {
    fn default() -> Self {
        Self {
            timeout_ms: default_tts_timeout_ms(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpeechOutputConfig {
    #[serde(default = "default_speech_min_chars")]
    pub min_chars: usize,
    #[serde(default = "default_speech_soft_break_min_chars")]
    pub soft_break_min_chars: usize,
    #[serde(default = "default_speech_max_chars")]
    pub max_chars: usize,
    #[serde(default = "default_pending_segments")]
    pub pending_segments: usize,
}

impl Default for SpeechOutputConfig {
    fn default() -> Self {
        Self {
            min_chars: default_speech_min_chars(),
            soft_break_min_chars: default_speech_soft_break_min_chars(),
            max_chars: default_speech_max_chars(),
            pending_segments: default_pending_segments(),
        }
    }
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            max_history_messages: default_max_history_messages(),
        }
    }
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
    pub urgent_control_queue: usize,
    #[serde(default = "default_queue_capacity")]
    pub outbound_audio_queue: usize,
    #[serde(default = "default_max_active_turns")]
    pub max_active_turns: usize,
    #[serde(default = "default_llm_concurrency")]
    pub llm_concurrency: usize,
    #[serde(default = "default_tts_concurrency")]
    pub tts_concurrency: usize,
}

impl Default for LimitsConfig {
    fn default() -> Self {
        Self {
            session_event_queue: default_queue_capacity(),
            outbound_control_queue: default_queue_capacity(),
            urgent_control_queue: default_queue_capacity(),
            outbound_audio_queue: default_queue_capacity(),
            max_active_turns: default_max_active_turns(),
            llm_concurrency: default_llm_concurrency(),
            tts_concurrency: default_tts_concurrency(),
        }
    }
}
