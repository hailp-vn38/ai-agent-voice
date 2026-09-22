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
    #[serde(default)]
    pub tts: TtsConfig,
    #[serde(default)]
    pub speech_output: SpeechOutputConfig,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
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
}

impl Default for ZeroTtsOnnxConfig {
    fn default() -> Self {
        Self {
            model: default_tts_model(),
            num_threads: default_asr_threads(),
            voice: default_tts_voice(),
        }
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

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
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
            outbound_audio_queue: default_queue_capacity(),
            max_active_turns: default_max_active_turns(),
            llm_concurrency: default_llm_concurrency(),
            tts_concurrency: default_tts_concurrency(),
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
fn default_llm_concurrency() -> usize {
    2
}
fn default_llm_timeout_ms() -> u64 {
    60_000
}
fn default_tts_concurrency() -> usize {
    2
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
fn default_llm_adapter() -> String {
    "openai".into()
}
fn default_openai_base_url() -> Url {
    Url::parse("https://api.openai.com/v1").expect("valid default OpenAI URL")
}
fn default_openai_model() -> String {
    "model-name".into()
}
fn default_tts_adapter() -> String {
    "zerotts_onnx".into()
}
fn default_tts_model() -> String {
    "zerotts_default".into()
}
fn default_tts_voice() -> String {
    "maichi".into()
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
fn default_tts_timeout_ms() -> u64 {
    15_000
}
fn default_speech_min_chars() -> usize {
    24
}
fn default_speech_soft_break_min_chars() -> usize {
    48
}
fn default_speech_max_chars() -> usize {
    160
}
fn default_pending_segments() -> usize {
    8
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
            self.limits.llm_concurrency,
            self.limits.tts_concurrency,
        ]
        .contains(&0)
        {
            return Err(ConfigError::Validation(
                "queue and delivery capacities must be positive".into(),
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
            || self.workers.tts.max_workers == 0
            || self.workers.tts.command_queue_capacity == 0
            || self.workers.tts.cleanup_grace_ms == 0
        {
            return Err(ConfigError::Validation(
                "VAD/ASR/TTS worker capacities and timeouts must be positive".into(),
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
        if vad.model.is_empty() || asr.model.is_empty() {
            return Err(ConfigError::Validation(
                "provider model identities must be non-empty".into(),
            ));
        }
        crate::providers::compiled_provider_registry()
            .llm_factory(&self.providers.llm.adapter)
            .map_err(|_| {
                ConfigError::Validation(format!(
                    "LLM adapter `{}` is not compiled into this binary",
                    self.providers.llm.adapter
                ))
            })?;
        let openai = self.providers.llm.openai.as_ref().ok_or_else(|| {
            ConfigError::Validation(format!(
                "providers.llm.{} options are required",
                self.providers.llm.adapter
            ))
        })?;
        let local_http = openai.base_url.scheme() == "http"
            && matches!(
                openai.base_url.host_str(),
                Some("localhost") | Some("127.0.0.1") | Some("::1")
            );
        if (openai.base_url.scheme() != "https" && !local_http)
            || openai.base_url.host_str().is_none()
            || openai.model.trim().is_empty()
            || openai.timeout_ms == 0
        {
            return Err(ConfigError::Validation(
                "OpenAI base URL and model must be valid".into(),
            ));
        }
        crate::providers::compiled_provider_registry()
            .tts_factory(&self.providers.tts.adapter)
            .map_err(|_| {
                ConfigError::Validation(format!(
                    "TTS adapter `{}` is not compiled into this binary",
                    self.providers.tts.adapter
                ))
            })?;
        let tts = self.providers.tts.zerotts_onnx.as_ref().ok_or_else(|| {
            ConfigError::Validation(format!(
                "providers.tts.{} options are required",
                self.providers.tts.adapter
            ))
        })?;
        if tts.model.trim().is_empty() || tts.voice.trim().is_empty() || tts.num_threads <= 0 {
            return Err(ConfigError::Validation(
                "ZeroTTS model, voice, and thread count must be valid".into(),
            ));
        }
        if self.limits.tts_concurrency != self.workers.tts.max_workers {
            return Err(ConfigError::Validation(
                "limits.tts_concurrency must equal workers.tts.max_workers".into(),
            ));
        }
        if self.tts.timeout_ms == 0
            || self.speech_output.min_chars == 0
            || self.speech_output.min_chars > self.speech_output.soft_break_min_chars
            || self.speech_output.soft_break_min_chars > self.speech_output.max_chars
            || self.speech_output.pending_segments == 0
            || self.speech_output.pending_segments > 64
        {
            return Err(ConfigError::Validation(
                "SpeechOutput bounds and TTS timeout must be valid".into(),
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
