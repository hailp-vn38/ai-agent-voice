use super::AppConfig;
use std::{fs, path::Path, str::FromStr};
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
        validate_providers(self)?;
        match target {
            BenchmarkTarget::AsrProvider
            | BenchmarkTarget::LlmProvider
            | BenchmarkTarget::TtsProvider => Ok(()),
            BenchmarkTarget::TtsDelivery => validate_audio(self),
            BenchmarkTarget::VadProvider => Ok(()),
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
        validate_database(self)?;
        validate_admin_api(self)?;
        validate_shutdown(self)?;
        Ok(())
    }

    pub fn max_capture_frames(&self) -> usize {
        (self.audio.max_utterance_ms / u64::from(self.audio.frame_ms)) as usize
    }
}

fn validate_admin_api(config: &AppConfig) -> Result<(), ConfigError> {
    if config.api.enabled && (!config.database.enabled || config.api.admin_token.trim().is_empty())
    {
        return Err(ConfigError::Validation(
            "api.enabled requires database.enabled and a non-empty admin_token".into(),
        ));
    }
    Ok(())
}

fn validate_database(config: &AppConfig) -> Result<(), ConfigError> {
    let database = &config.database;
    if database.devices.admission_enabled && !database.enabled {
        return Err(ConfigError::Validation(
            "database.devices.admission_enabled requires database.enabled".into(),
        ));
    }
    if database.devices.auto_register
        && (!database.devices.admission_enabled
            || database.devices.auto_register_agent_key.trim().is_empty())
    {
        return Err(ConfigError::Validation(
            "database.devices.auto_register requires admission_enabled and a non-empty auto_register_agent_key".into(),
        ));
    }
    if !database.enabled {
        return Ok(());
    }
    let has_memory_mode = url::Url::parse(&database.url).is_ok_and(|url| {
        url.query_pairs()
            .any(|(key, value)| key == "mode" && value == "memory")
    });
    if !(1..=32).contains(&database.max_connections)
        || !(1..=30_000).contains(&database.busy_timeout_ms)
        || database.url.contains(":memory:")
        || has_memory_mode
        || !database.url.starts_with("sqlite://")
        || sqlx::sqlite::SqliteConnectOptions::from_str(&database.url).is_err()
    {
        return Err(ConfigError::Validation(
            "database requires a local sqlite URL, 1..=32 pool connections, and busy_timeout_ms 1..=30000".into(),
        ));
    }
    Ok(())
}

fn validate_shutdown(config: &AppConfig) -> Result<(), ConfigError> {
    if !(1_000..=60_000).contains(&config.shutdown.grace_ms) {
        return Err(ConfigError::Validation(
            "shutdown.grace_ms must be 1000..=60000".into(),
        ));
    }
    Ok(())
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
        config.limits.vision_concurrency,
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
    let network = &config.mcp.external.network;
    if network.allowed_hosts.iter().any(|host| {
        host.is_empty()
            || host.len() > 253
            || !host
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'*'))
    }) || network.allowed_cidrs.iter().any(|cidr| !valid_cidr(cidr))
    {
        return Err(ConfigError::Validation(
            "External MCP network allowlist must contain valid host patterns and CIDRs".into(),
        ));
    }
    Ok(())
}

fn valid_cidr(value: &str) -> bool {
    let Some((address, prefix)) = value.split_once('/') else {
        return false;
    };
    let Ok(address) = address.parse::<std::net::IpAddr>() else {
        return false;
    };
    prefix
        .parse::<u8>()
        .is_ok_and(|prefix| prefix <= if address.is_ipv4() { 32 } else { 128 })
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
    use crate::config::TtsInstanceConfig;
    let registry = crate::providers::compiled_provider_registry();
    for (id, instance) in &config.providers.vad.instances {
        validate_instance_id(id)?;
        registry.vad_factory(instance.adapter()).map_err(|_| {
            ConfigError::Validation(format!(
                "VAD instance `{id}` uses adapter `{}` which is not compiled into this binary",
                instance.adapter()
            ))
        })?;
        let vad = instance.silero_onnx();
        if vad.min_speech_ms == 0
            || vad.end_silence_ms == 0
            || vad.pre_roll_ms > config.audio.max_utterance_ms
            || vad.num_threads <= 0
            || !vad.speech_threshold.is_finite()
            || !vad.exit_threshold.is_finite()
            || !(0.0..=1.0).contains(&vad.exit_threshold)
            || vad.exit_threshold >= vad.speech_threshold
            || vad.speech_threshold > 1.0
            || vad.model.trim().is_empty()
        {
            return Err(ConfigError::Validation(format!(
                "VAD instance `{id}` has invalid thresholds, durations, or model identity"
            )));
        }
    }
    for (id, instance) in &config.providers.asr.instances {
        validate_instance_id(id)?;
        registry.asr_factory(instance.adapter()).map_err(|_| {
            ConfigError::Validation(format!(
                "ASR instance `{id}` uses adapter `{}` which is not compiled into this binary",
                instance.adapter()
            ))
        })?;
        match instance {
            crate::config::AsrInstanceConfig::ZipformerSherpa(asr)
                if asr.num_threads <= 0
                    || asr.decoding_method.trim().is_empty()
                    || asr.model.trim().is_empty() =>
            {
                return Err(ConfigError::Validation(format!(
                    "ASR instance `{id}` has invalid runtime options or model identity"
                )));
            }
            crate::config::AsrInstanceConfig::GipformerSherpaOffline(asr)
                if asr.model.trim().is_empty()
                    || asr.num_threads <= 0
                    || !matches!(
                        asr.decoding_method.as_str(),
                        "greedy_search" | "modified_beam_search"
                    )
                    || asr.max_active_paths <= 0 =>
            {
                return Err(ConfigError::Validation(format!(
                    "Gipformer ASR instance `{id}` has invalid model identity or runtime options"
                )));
            }
            _ => {}
        }
    }
    for (id, instance) in &config.providers.llm.instances {
        validate_instance_id(id)?;
        registry.llm_factory(instance.adapter()).map_err(|_| {
            ConfigError::Validation(format!(
                "LLM instance `{id}` uses adapter `{}` which is not compiled into this binary",
                instance.adapter()
            ))
        })?;
        let openai = instance.openai();
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
            return Err(ConfigError::Validation(format!(
                "LLM instance `{id}` has invalid OpenAI URL, model, or timeout"
            )));
        }
    }
    for (id, instance) in &config.providers.tts.instances {
        validate_instance_id(id)?;
        registry.tts_factory(instance.adapter()).map_err(|_| {
            ConfigError::Validation(format!(
                "TTS instance `{id}` uses adapter `{}` which is not compiled into this binary",
                instance.adapter()
            ))
        })?;
        match instance {
            TtsInstanceConfig::ZeroTtsOnnx(tts)
                if tts.model.trim().is_empty()
                    || tts.voice.trim().is_empty()
                    || tts.num_threads <= 0 =>
            {
                return Err(ConfigError::Validation(format!(
                    "ZeroTTS instance `{id}` has invalid model, voice, or thread count"
                )));
            }
            TtsInstanceConfig::ChillAudioWs(tts)
                if tts.ws_url.scheme() != "wss"
                    || tts.ws_url.host_str().is_none()
                    || tts.app_key.expose().trim().is_empty()
                    || tts.token.expose().trim().is_empty()
                    || tts.voice.trim().is_empty()
                    || tts.timeout_ms == 0 =>
            {
                return Err(ConfigError::Validation(format!(
                    "ChillAudio instance `{id}` has invalid URL, credential, voice, or timeout"
                )));
            }
            _ => {}
        }
    }
    for (id, instance) in &config.providers.vision.instances {
        validate_instance_id(id)?;
        registry.vision_factory(instance.adapter()).map_err(|_| {
            ConfigError::Validation(format!(
                "VISION instance `{id}` uses adapter `{}` which is not compiled into this binary",
                instance.adapter()
            ))
        })?;
        let vision = instance.openai_vision();
        if !matches!(vision.base_url.scheme(), "http" | "https")
            || vision.base_url.host_str().is_none()
            || vision.model.trim().is_empty()
            || vision.timeout_ms == 0
            || vision.max_tokens == 0
            || !vision.temperature.is_finite()
            || !(0.0..=2.0).contains(&vision.temperature)
            || !vision.top_p.is_finite()
            || !(0.0..=1.0).contains(&vision.top_p)
            || vision.top_p == 0.0
        {
            return Err(ConfigError::Validation(format!(
                "VISION instance `{id}` has invalid OpenAI-compatible options"
            )));
        }
    }
    let defaults = &config.provider_defaults;
    require_instance(
        "default VAD",
        &defaults.vad,
        &config.providers.vad.instances,
    )?;
    require_instance(
        "default ASR",
        &defaults.asr,
        &config.providers.asr.instances,
    )?;
    require_instance(
        "default LLM",
        &defaults.llm,
        &config.providers.llm.instances,
    )?;
    require_instance(
        "default TTS",
        &defaults.tts,
        &config.providers.tts.instances,
    )?;
    let bindings = &config.effective_agent.providers;
    require_instance("VAD", &bindings.vad, &config.providers.vad.instances)?;
    require_instance("ASR", &bindings.asr, &config.providers.asr.instances)?;
    require_instance("LLM", &bindings.llm, &config.providers.llm.instances)?;
    require_instance("TTS", &bindings.tts, &config.providers.tts.instances)?;
    validate_vision(config)
}

fn validate_vision(config: &AppConfig) -> Result<(), ConfigError> {
    const HARD_MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;
    let vision = &config.vision;
    if vision.max_image_bytes == 0
        || vision.max_image_bytes > HARD_MAX_IMAGE_BYTES
        || vision.max_question_bytes == 0
    {
        return Err(ConfigError::Validation(
            "Vision image and question limits must be within supported bounds".into(),
        ));
    }
    if !vision.enabled {
        return Ok(());
    }
    let binding = config
        .effective_agent
        .providers
        .vision
        .as_deref()
        .ok_or_else(|| {
            ConfigError::Validation(
                "vision.enabled requires an effective Vision provider binding".into(),
            )
        })?;
    require_instance("VISION", binding, &config.providers.vision.instances)?;
    if vision.advertise_via_mcp {
        let public_url = vision.public_url.as_ref().ok_or_else(|| {
            ConfigError::Validation("vision.advertise_via_mcp requires vision.public_url".into())
        })?;
        if public_url.path() != "/mcp/vision/explain" || public_url.query().is_some() {
            return Err(ConfigError::Validation(
                "vision.public_url must be the Vision endpoint and must not contain query parameters".into(),
            ));
        }
        if config.auth.token.is_empty() {
            return Err(ConfigError::Validation(
                "vision.advertise_via_mcp requires auth.token".into(),
            ));
        }
    }
    Ok(())
}

fn validate_instance_id(id: &str) -> Result<(), ConfigError> {
    if id.is_empty()
        || !id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
    {
        return Err(ConfigError::Validation(format!(
            "provider instance id `{id}` is invalid"
        )));
    }
    Ok(())
}

fn require_instance<T>(
    kind: &str,
    id: &str,
    instances: &std::collections::BTreeMap<String, T>,
) -> Result<(), ConfigError> {
    if instances.contains_key(id) {
        Ok(())
    } else {
        Err(ConfigError::Validation(format!(
            "agent {kind} provider `{id}` does not exist"
        )))
    }
}

fn validate_speech_output(config: &AppConfig) -> Result<(), ConfigError> {
    let output = &config.speech_output;
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
