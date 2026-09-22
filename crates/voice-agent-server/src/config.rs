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
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProvidersConfig {
    pub vad: VadProviderConfig,
    pub asr: AsrProviderConfig,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VadProviderConfig {
    #[serde(default = "default_vad_adapter")]
    pub adapter: String,
    #[serde(default = "default_vad_model")]
    pub model: String,
    #[serde(default)]
    pub silero_onnx: Option<SileroOnnxConfig>,
}

impl Default for VadProviderConfig {
    fn default() -> Self {
        Self {
            adapter: default_vad_adapter(),
            model: default_vad_model(),
            silero_onnx: Some(SileroOnnxConfig::default()),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SileroOnnxConfig {
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
    #[serde(default = "default_asr_model")]
    pub model: String,
    #[serde(default)]
    pub zipformer_sherpa: Option<ZipformerSherpaConfig>,
}

impl Default for AsrProviderConfig {
    fn default() -> Self {
        Self {
            adapter: default_asr_adapter(),
            model: default_asr_model(),
            zipformer_sherpa: Some(ZipformerSherpaConfig::default()),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ZipformerSherpaConfig {
    #[serde(default = "default_asr_threads")]
    pub num_threads: i32,
    #[serde(default = "default_decoding_method")]
    pub decoding_method: String,
}

impl Default for ZipformerSherpaConfig {
    fn default() -> Self {
        Self {
            num_threads: default_asr_threads(),
            decoding_method: default_decoding_method(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkersConfig {
    #[serde(default)]
    pub vad: VadWorkerConfig,
    #[serde(default)]
    pub asr: AsrWorkerConfig,
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
    pub outbound_audio_queue: usize,
    #[serde(default = "default_max_active_turns")]
    pub max_active_turns: usize,
}

impl Default for LimitsConfig {
    fn default() -> Self {
        Self {
            session_event_queue: default_queue_capacity(),
            outbound_control_queue: default_queue_capacity(),
            outbound_audio_queue: default_queue_capacity(),
            max_active_turns: default_max_active_turns(),
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
fn default_max_active_turns() -> usize {
    8
}
fn default_vad_adapter() -> String {
    "silero_onnx".into()
}
fn default_vad_model() -> String {
    "silero_vad_v5".into()
}
fn default_asr_adapter() -> String {
    "zipformer_sherpa".into()
}
fn default_asr_model() -> String {
    "zipformer_vi_streaming".into()
}
fn default_provider_threads() -> i32 {
    1
}
fn default_min_speech_ms() -> u64 {
    180
}
fn default_end_silence_ms() -> u64 {
    600
}
fn default_pre_roll_ms() -> u64 {
    300
}
fn default_speech_threshold() -> f32 {
    0.50
}
fn default_exit_threshold() -> f32 {
    0.35
}
fn default_vad_worker_count() -> usize {
    4
}
fn default_asr_worker_count() -> usize {
    2
}
fn default_asr_threads() -> i32 {
    2
}
fn default_decoding_method() -> String {
    "greedy_search".into()
}
fn default_manifest_path() -> std::path::PathBuf {
    "models/manifest.toml".into()
}
fn default_models_root() -> std::path::PathBuf {
    "models".into()
}
fn default_onnx_runtime_library() -> std::path::PathBuf {
    "runtime/onnxruntime/libonnxruntime.dylib".into()
}
fn default_worker_queue_capacity() -> usize {
    32
}
fn default_asr_final_timeout_ms() -> u64 {
    15_000
}
fn default_vad_reset_timeout_ms() -> u64 {
    15_000
}
fn default_cleanup_grace_ms() -> u64 {
    5_000
}
fn default_max_history_messages() -> usize {
    20
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
            self.limits.max_active_turns,
        ]
        .contains(&0)
        {
            return Err(ConfigError::Validation(
                "queue capacities must be positive".into(),
            ));
        }
        if self.llm.max_history_messages == 0 {
            return Err(ConfigError::Validation(
                "llm.max_history_messages must be positive".into(),
            ));
        }
        if self.workers.asr.max_workers == 0
            || self.workers.asr.command_queue_capacity == 0
            || self.workers.asr.final_timeout_ms == 0
            || self.workers.asr.cleanup_grace_ms == 0
            || self.workers.vad.max_workers == 0
            || self.workers.vad.command_queue_capacity == 0
            || self.workers.vad.reset_timeout_ms == 0
            || self.workers.vad.cleanup_grace_ms == 0
        {
            return Err(ConfigError::Validation(
                "VAD/ASR worker capacities and timeouts must be positive".into(),
            ));
        }
        crate::providers::compiled_provider_registry()
            .vad_factory(&self.providers.vad.adapter)
            .map_err(|_| {
                ConfigError::Validation(format!(
                    "VAD adapter `{}` is not compiled into this binary",
                    self.providers.vad.adapter
                ))
            })?;
        let vad = self.providers.vad.silero_onnx.as_ref().ok_or_else(|| {
            ConfigError::Validation(format!(
                "providers.vad.{} options are required",
                self.providers.vad.adapter
            ))
        })?;
        if vad.min_speech_ms == 0
            || vad.end_silence_ms == 0
            || vad.pre_roll_ms > self.audio.max_utterance_ms
            || vad.num_threads <= 0
            || !vad.speech_threshold.is_finite()
            || !vad.exit_threshold.is_finite()
            || !(0.0..=1.0).contains(&vad.exit_threshold)
            || vad.exit_threshold >= vad.speech_threshold
            || vad.speech_threshold > 1.0
        {
            return Err(ConfigError::Validation(
                "VAD thresholds and segmentation durations must be valid".into(),
            ));
        }
        crate::providers::compiled_provider_registry()
            .asr_factory(&self.providers.asr.adapter)
            .map_err(|_| {
                ConfigError::Validation(format!(
                    "ASR adapter `{}` is not compiled into this binary",
                    self.providers.asr.adapter
                ))
            })?;
        let asr = self
            .providers
            .asr
            .zipformer_sherpa
            .as_ref()
            .ok_or_else(|| {
                ConfigError::Validation(format!(
                    "providers.asr.{} options are required",
                    self.providers.asr.adapter
                ))
            })?;
        if asr.num_threads <= 0 || asr.decoding_method.is_empty() {
            return Err(ConfigError::Validation(
                "ASR runtime options must be valid".into(),
            ));
        }
        if self.providers.vad.model.is_empty() || self.providers.asr.model.is_empty() {
            return Err(ConfigError::Validation(
                "provider model identities must be non-empty".into(),
            ));
        }
        if self.deployment.profile.is_empty()
            || self.deployment.model_manifest.as_os_str().is_empty()
            || self.deployment.models.root.as_os_str().is_empty()
        {
            return Err(ConfigError::Validation(
                "deployment profile and model manifest must be set".into(),
            ));
        }
        if self.runtime.onnx.library.as_os_str().is_empty() {
            return Err(ConfigError::Validation(
                "runtime.onnx.library must be set".into(),
            ));
        }
        Ok(())
    }

    pub fn max_capture_frames(&self) -> usize {
        (self.audio.max_utterance_ms / u64::from(self.audio.frame_ms)) as usize
    }
}
