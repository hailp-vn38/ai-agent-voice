use super::AppConfig;
use std::{fs, path::Path};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("cannot read config: {0}")]
    Read(#[from] std::io::Error),
    #[error("invalid TOML: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("invalid configuration: {0}")]
    Validation(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BenchmarkTarget {
    AsrProvider,
    LlmProvider,
    TtsProvider,
    TtsDelivery,
    VadProvider,
}

impl AppConfig {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let config = Self::parse_and_resolve(path)?;
        config.validate()?;
        Ok(config)
    }

    pub fn parse_and_resolve(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let path = path.as_ref();
        let mut config: Self = toml::from_str(&fs::read_to_string(path)?)?;
        config.resolve_agent(path)?;
        Ok(config)
    }

    pub fn load_for_benchmark(
        path: impl AsRef<Path>,
        target: BenchmarkTarget,
    ) -> Result<Self, ConfigError> {
        let config = Self::parse_and_resolve(path)?;
        config.validate_for_benchmark(target)?;
        Ok(config)
    }

    pub fn validate_for_benchmark(&self, target: BenchmarkTarget) -> Result<(), ConfigError> {
        validate_deployment(self)?;
        match target {
            BenchmarkTarget::AsrProvider => validate_asr_provider(self),
            BenchmarkTarget::LlmProvider => validate_llm_provider(self),
            BenchmarkTarget::TtsProvider => validate_tts_provider(self),
            BenchmarkTarget::TtsDelivery => {
                validate_tts_provider(self)?;
                validate_audio(self)
            }
            BenchmarkTarget::VadProvider => validate_vad_provider(self),
        }
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        validate_transport(self)?;
        validate_audio(self)?;
        validate_capacity(self)?;
        validate_workers(self)?;
        validate_providers(self)?;
        validate_speech_output(self)?;
        validate_deployment(self)?;
        Ok(())
    }

    pub fn max_capture_frames(&self) -> usize {
        (self.audio.max_utterance_ms / u64::from(self.audio.frame_ms)) as usize
    }
}

fn validate_transport(config: &AppConfig) -> Result<(), ConfigError> {
    if !matches!(config.server.public_ws_url.scheme(), "ws" | "wss") {
        return Err(ConfigError::Validation(
            "server.public_ws_url must use ws or wss".into(),
        ));
    }
    if config.server.hello_timeout_ms == 0
        || !(4_000..=1_048_576).contains(&config.websocket.max_frame_bytes)
    {
        return Err(ConfigError::Validation(
            "hello timeout and WebSocket maximum frame size must be valid".into(),
        ));
    }
    Ok(())
}

fn validate_audio(config: &AppConfig) -> Result<(), ConfigError> {
    let audio = &config.audio;
    if audio.input_sample_rate != 16_000
        || audio.output_sample_rate != 24_000
        || audio.channels != 1
        || audio.frame_ms != 60
    {
        return Err(ConfigError::Validation(
            "V1 requires canonical audio: uplink 16 kHz, downlink 24 kHz, mono, 60 ms".into(),
        ));
    }
    if !(1_000..=120_000).contains(&audio.max_utterance_ms)
        || !audio
            .max_utterance_ms
            .is_multiple_of(u64::from(audio.frame_ms))
    {
        return Err(ConfigError::Validation(
            "audio.max_utterance_ms must be 1000..=120000 and divisible by frame_ms".into(),
        ));
    }
    Ok(())
}

fn validate_capacity(config: &AppConfig) -> Result<(), ConfigError> {
    if [
        config.limits.session_event_queue,
        config.limits.outbound_control_queue,
        config.limits.urgent_control_queue,
        config.limits.outbound_audio_queue,
        config.limits.max_active_turns,
        config.limits.llm_concurrency,
        config.limits.tts_concurrency,
    ]
    .contains(&0)
    {
        return Err(ConfigError::Validation(
            "queue and delivery capacities must be positive".into(),
        ));
    }
    if config.llm.max_history_messages == 0
        || config.llm.prompt_budget_tokens == 0
        || config.llm.max_tool_result_chars == 0
        || config.llm.max_tool_depth == 0
    {
        return Err(ConfigError::Validation(
            "LLM history, prompt budget and tool limits must be positive".into(),
        ));
    }
    if config.mcp.call_timeout_ms == 0
        || config.mcp.discovery_timeout_ms == 0
        || config
            .mcp
            .allowed_tools
            .iter()
            .any(|name| name.trim().is_empty())
        || {
            let mut names = std::collections::HashSet::new();
            !config
                .mcp
                .allowed_tools
                .iter()
                .all(|name| names.insert(name))
        }
    {
        return Err(ConfigError::Validation(
            "MCP timeouts and allowlist must be valid".into(),
        ));
    }
    if config
        .mcp
        .allowed_tools
        .iter()
        .any(|name| crate::tools::device_mcp::is_dangerous_tool(name))
    {
        return Err(ConfigError::Validation(
            "MCP allowlist must not contain dangerous tools".into(),
        ));
    }
    let mut policy_names = std::collections::HashSet::new();
    if config.mcp.tool_policy.iter().any(|policy| {
        policy.name.trim().is_empty()
            || crate::tools::device_mcp::is_dangerous_tool(&policy.name)
            || !policy_names.insert(&policy.name)
    }) {
        return Err(ConfigError::Validation(
            "MCP tool policy names must be unique, non-empty, and non-dangerous".into(),
        ));
    }
    Ok(())
}

fn validate_workers(config: &AppConfig) -> Result<(), ConfigError> {
    let workers = &config.workers;
    if workers.asr.max_workers == 0
        || workers.asr.command_queue_capacity == 0
        || workers.asr.final_timeout_ms == 0
        || workers.asr.cleanup_grace_ms == 0
        || workers.vad.max_workers == 0
        || workers.vad.command_queue_capacity == 0
        || workers.vad.reset_timeout_ms == 0
        || workers.vad.cleanup_grace_ms == 0
        || workers.tts.max_workers == 0
        || workers.tts.command_queue_capacity == 0
        || workers.tts.cleanup_grace_ms == 0
    {
        return Err(ConfigError::Validation(
            "VAD/ASR/TTS worker capacities and timeouts must be positive".into(),
        ));
    }
    Ok(())
}

fn validate_providers(config: &AppConfig) -> Result<(), ConfigError> {
    validate_vad_provider(config)?;
    validate_asr_provider(config)?;
    validate_llm_provider(config)?;
    validate_tts_provider(config)
}

fn validate_llm_provider(config: &AppConfig) -> Result<(), ConfigError> {
    let registry = crate::providers::compiled_provider_registry();
    registry
        .llm_factory(&config.providers.llm.adapter)
        .map_err(|_| {
            ConfigError::Validation(format!(
                "LLM adapter `{}` is not compiled into this binary",
                config.providers.llm.adapter
            ))
        })?;
    let openai = config.providers.llm.openai.as_ref().ok_or_else(|| {
        ConfigError::Validation(format!(
            "providers.llm.{} options are required",
            config.providers.llm.adapter
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
    Ok(())
}

fn validate_vad_provider(config: &AppConfig) -> Result<(), ConfigError> {
    let registry = crate::providers::compiled_provider_registry();
    registry
        .vad_factory(&config.providers.vad.adapter)
        .map_err(|_| {
            ConfigError::Validation(format!(
                "VAD adapter `{}` is not compiled into this binary",
                config.providers.vad.adapter
            ))
        })?;
    let vad = config.providers.vad.silero_onnx.as_ref().ok_or_else(|| {
        ConfigError::Validation(format!(
            "providers.vad.{} options are required",
            config.providers.vad.adapter
        ))
    })?;
    if vad.min_speech_ms == 0
        || vad.end_silence_ms == 0
        || vad.pre_roll_ms > config.audio.max_utterance_ms
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
    if vad.model.is_empty() {
        return Err(ConfigError::Validation(
            "VAD provider model identity must be non-empty".into(),
        ));
    }
    Ok(())
}

fn validate_asr_provider(config: &AppConfig) -> Result<(), ConfigError> {
    let registry = crate::providers::compiled_provider_registry();
    registry
        .asr_factory(&config.providers.asr.adapter)
        .map_err(|_| {
            ConfigError::Validation(format!(
                "ASR adapter `{}` is not compiled into this binary",
                config.providers.asr.adapter
            ))
        })?;
    let asr = config
        .providers
        .asr
        .zipformer_sherpa
        .as_ref()
        .ok_or_else(|| {
            ConfigError::Validation(format!(
                "providers.asr.{} options are required",
                config.providers.asr.adapter
            ))
        })?;
    if asr.num_threads <= 0 || asr.decoding_method.is_empty() {
        return Err(ConfigError::Validation(
            "ASR runtime options must be valid".into(),
        ));
    }
    if asr.model.is_empty() {
        return Err(ConfigError::Validation(
            "ASR provider model identity must be non-empty".into(),
        ));
    }
    Ok(())
}

fn validate_tts_provider(config: &AppConfig) -> Result<(), ConfigError> {
    let registry = crate::providers::compiled_provider_registry();
    registry
        .tts_factory(&config.providers.tts.adapter)
        .map_err(|_| {
            ConfigError::Validation(format!(
                "TTS adapter `{}` is not compiled into this binary",
                config.providers.tts.adapter
            ))
        })?;
    match config.providers.tts.adapter.as_str() {
        "zerotts_onnx" => {
            let tts = config.providers.tts.zerotts_onnx.as_ref().ok_or_else(|| {
                ConfigError::Validation("providers.tts.zerotts_onnx options are required".into())
            })?;
            if tts.model.trim().is_empty() || tts.voice.trim().is_empty() || tts.num_threads <= 0 {
                return Err(ConfigError::Validation(
                    "ZeroTTS model, voice, and thread count must be valid".into(),
                ));
            }
        }
        "chillaudio_ws" => {
            let tts = config.providers.tts.chillaudio_ws.as_ref().ok_or_else(|| {
                ConfigError::Validation("providers.tts.chillaudio_ws options are required".into())
            })?;
            if tts.ws_url.scheme() != "wss"
                || tts.ws_url.host_str().is_none()
                || tts.app_key.expose().trim().is_empty()
                || tts.token.expose().trim().is_empty()
                || tts.voice.trim().is_empty()
                || tts.timeout_ms == 0
            {
                return Err(ConfigError::Validation(
                    "ChillAudio WebSocket URL, credential, voice, and timeout must be valid".into(),
                ));
            }
        }
        _ => {}
    }
    Ok(())
}

fn validate_speech_output(config: &AppConfig) -> Result<(), ConfigError> {
    let output = &config.speech_output;
    if config.limits.tts_concurrency != config.workers.tts.max_workers {
        return Err(ConfigError::Validation(
            "limits.tts_concurrency must equal workers.tts.max_workers".into(),
        ));
    }
    if config.tts.timeout_ms == 0
        || output.min_chars == 0
        || output.min_chars > output.soft_break_min_chars
        || output.soft_break_min_chars > output.max_chars
        || output.pending_segments == 0
        || output.pending_segments > 64
    {
        return Err(ConfigError::Validation(
            "SpeechOutput bounds and TTS timeout must be valid".into(),
        ));
    }
    Ok(())
}

fn validate_deployment(config: &AppConfig) -> Result<(), ConfigError> {
    if config.deployment.profile.is_empty()
        || config.deployment.model_manifest.as_os_str().is_empty()
        || config.deployment.models.root.as_os_str().is_empty()
    {
        return Err(ConfigError::Validation(
            "deployment profile and model manifest must be set".into(),
        ));
    }
    if config.runtime.onnx.library.as_os_str().is_empty() {
        return Err(ConfigError::Validation(
            "runtime.onnx.library must be set".into(),
        ));
    }
    Ok(())
}
