use super::AppConfig;
use std::{fs, net::IpAddr, path::Path, str::FromStr};
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
        validate_speaker_recognition(self)?;
        validate_shutdown(self)?;
        if let Some(runtime) = &self.provider_runtime {
            if !(1000..=120_000).contains(&runtime.startup_timeout_ms) {
                return Err(ConfigError::Validation(
                    "invalid provider startup timeout".into(),
                ));
            }
            runtime
                .limits
                .validate()
                .map_err(|_| ConfigError::Validation("invalid provider runtime limits".into()))?;
            if runtime.estimated_peak_bytes.is_empty()
                || runtime.estimated_peak_bytes.len() > 64
                || runtime
                    .estimated_peak_bytes
                    .values()
                    .any(|bytes| *bytes == 0 || *bytes > runtime.limits.max_resident_bytes)
            {
                return Err(ConfigError::Validation(
                    "invalid provider runtime measured estimates".into(),
                ));
            }
            if self.vision.enabled {
                return Err(ConfigError::Validation(
                    "managed provider startup does not support Vision".into(),
                ));
            }
        }
        Ok(())
    }

    pub fn max_capture_frames(&self) -> usize {
        (self.audio.max_utterance_ms / u64::from(self.audio.frame_ms)) as usize
    }
}

fn validate_admin_api(config: &AppConfig) -> Result<(), ConfigError> {
    if config.api.enabled && config.api.admin_token.trim().is_empty() {
        return Err(ConfigError::Validation(
            "api.enabled requires a non-empty admin_token".into(),
        ));
    }
    if !(1..=8).contains(&config.api.mcp_tests.max_concurrency)
        || !(1000..=120_000).contains(&config.api.mcp_tests.timeout_ms)
    {
        return Err(ConfigError::Validation(
            "api.mcp_tests bounds are invalid".into(),
        ));
    }
    if !(1..=8).contains(&config.api.provider_tests.max_concurrency)
        || !(1_000..=120_000).contains(&config.api.provider_tests.timeout_ms)
    {
        return Err(ConfigError::Validation(
            "api.provider_tests bounds are invalid".into(),
        ));
    }
    Ok(())
}

fn validate_speaker_recognition(config: &AppConfig) -> Result<(), ConfigError> {
    let speaker = &config.speaker_recognition;
    if !speaker.similarity_threshold.is_finite()
        || !(0.0..=1.0).contains(&speaker.similarity_threshold)
        || speaker.similarity_threshold == 0.0
        || !(1..=4096).contains(&speaker.max_speakers)
        || !(1..=256).contains(&speaker.max_candidates_per_agent)
        || !(1..=16).contains(&speaker.max_voiceprint_spaces_per_speaker)
    {
        return Err(ConfigError::Validation(
            "speaker_recognition bounds are invalid".into(),
        ));
    }
    let enrollment = &speaker.enrollment;
    if !(1..=8).contains(&enrollment.min_samples)
        || !(enrollment.min_samples..=8).contains(&enrollment.max_samples)
        || !(500..=60_000).contains(&enrollment.min_clip_ms)
        || !(enrollment.min_clip_ms..=60_000).contains(&enrollment.max_clip_ms)
        || !(250..=60_000).contains(&enrollment.min_speech_ms)
        || enrollment.min_speech_ms > enrollment.max_clip_ms
        || !(60_000..=86_400_000).contains(&enrollment.ttl_ms)
        || !(1..=256).contains(&enrollment.max_open_enrollments)
        || !(16 * 1024..=4 * 1024 * 1024).contains(&enrollment.max_audio_body_bytes)
    {
        return Err(ConfigError::Validation(
            "speaker_recognition.enrollment bounds are invalid".into(),
        ));
    }
    Ok(())
}

fn validate_database(config: &AppConfig) -> Result<(), ConfigError> {
    let database = &config.database;
    if database.devices.auto_register && database.devices.auto_register_agent_key.trim().is_empty()
    {
        return Err(ConfigError::Validation(
            "database.devices.auto_register requires a non-empty auto_register_agent_key".into(),
        ));
    }
    let enrollment = &database.devices.enrollment;
    if enrollment.enabled {
        if !config.api.enabled {
            return Err(ConfigError::Validation(
                "database.devices.enrollment.enabled requires api.enabled".into(),
            ));
        }
        if database.devices.auto_register {
            return Err(ConfigError::Validation(
                "database.devices.enrollment.enabled requires database.devices.auto_register=false"
                    .into(),
            ));
        }
        if !(60..=3_600).contains(&enrollment.code_ttl_seconds)
            || !(enrollment.code_ttl_seconds..=604_800).contains(&enrollment.retention_seconds)
            || !(10..=3_600).contains(&enrollment.cleanup_interval_seconds)
            || !(1..=10_000).contains(&enrollment.max_pending)
            || !(1..=128).contains(&enrollment.ws_max_connections)
            || !(30..=600).contains(&enrollment.ws_timeout_seconds)
            || !(1_000..=10_000).contains(&enrollment.ws_poll_interval_ms)
            || !(30..=300).contains(&enrollment.ws_prompt_repeat_seconds)
            || enrollment.prompt_assets_dir.as_os_str().is_empty()
        {
            return Err(ConfigError::Validation(
                "database.devices.enrollment bounds are invalid".into(),
            ));
        }
    }
    // Retention is a property of the archive rather than of capture, so its bounds hold whether
    // capture is on or off.  `0` is refused alongside an out-of-range value: an unbounded
    // retention is not a retention policy, and there is no "keep forever" in V1.
    if !(1..=365).contains(&database.history.retention_days) {
        return Err(ConfigError::Validation(
            "database.history.retention_days must be 1..=365".into(),
        ));
    }
    if database.history.queue_capacity == 0 || database.history.queue_capacity > 65_536 {
        return Err(ConfigError::Validation(
            "database.history.queue_capacity must be 1..=65536".into(),
        ));
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
    {
        return Err(ConfigError::Validation(
            "LLM history, prompt budget and tool result limits must be positive".into(),
        ));
    }
    validate_tool_rounds(&config.llm.tools)?;
    if config.mcp.call_timeout_ms == 0 || config.mcp.discovery_timeout_ms == 0 {
        return Err(ConfigError::Validation("MCP timeouts must be valid".into()));
    }
    let mut policy_names = std::collections::HashSet::new();
    if config
        .mcp
        .tool_policy
        .iter()
        .any(|policy| policy.name.trim().is_empty() || !policy_names.insert(&policy.name))
    {
        return Err(ConfigError::Validation(
            "MCP tool policy names must be unique and non-empty".into(),
        ));
    }
    let network = &config.mcp.external.network;
    if network.allowed_hosts.iter().any(|host| {
        host.is_empty()
            || host.len() > 253
            || !host
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'*'))
    }) {
        return Err(ConfigError::Validation(
            "External MCP network allowlist must contain valid host patterns".into(),
        ));
    }
    validate_external_mcp(&config.mcp.external)
}

/// The Tool-round Executor's caps are the last thing an operator can widen, so each one is bounded
/// by a fixed hard ceiling here rather than trusted downstream.
///
/// A `0` is refused alongside an out-of-range value on purpose: these are policy caps, and a cap of
/// zero would mean the executor silently refuses every tool call rather than the deployment
/// failing loudly at startup.
pub(super) fn validate_tool_rounds(
    tools: &crate::config::LlmToolsConfig,
) -> Result<(), ConfigError> {
    if !(1..=32).contains(&tools.max_calls_per_round) {
        return Err(ConfigError::Validation(
            "Tool calls per round must be between 1 and 32".into(),
        ));
    }
    if !(1..=8).contains(&tools.max_rounds_per_turn) {
        return Err(ConfigError::Validation(
            "Tool rounds per turn must be between 1 and 8".into(),
        ));
    }
    if !(1..=120_000).contains(&tools.execution_budget_ms) {
        return Err(ConfigError::Validation(
            "Tool execution budget must be between 1 and 120000 milliseconds".into(),
        ));
    }
    Ok(())
}

/// External MCP budgets and caps are deployment policy, so every bound is checked once here,
/// before a listener binds, and never re-derived by the admission path.
fn validate_external_mcp(external: &crate::config::ExternalMcpConfig) -> Result<(), ConfigError> {
    let limits = &external.limits;
    if external.per_server_resolution_timeout_ms == 0
        || external.overall_resolution_budget_ms == 0
        || external.overall_resolution_budget_ms < external.per_server_resolution_timeout_ms
    {
        return Err(ConfigError::Validation(
            "External MCP resolution timeouts must be positive and the overall budget must not be smaller than one server timeout"
                .into(),
        ));
    }
    if !(1..=64).contains(&external.max_concurrent_calls_per_server) {
        return Err(ConfigError::Validation(
            "External MCP per-server call concurrency must be between 1 and 64".into(),
        ));
    }
    // Operator values are capped by fixed hard ceilings so persistent configuration can never
    // describe an unbounded catalog or an unbounded response body.
    if limits.max_tools_per_server == 0
        || limits.max_tools_per_server > 512
        || limits.max_tools_per_session == 0
        || limits.max_tools_per_session > 2_048
        || limits.max_tool_schema_bytes == 0
        || limits.max_tool_schema_bytes > 65_536
        || limits.max_tool_description_bytes == 0
        || limits.max_tool_description_bytes > 16_384
        || limits.max_external_tool_result_bytes == 0
        || limits.max_external_tool_result_bytes > 65_536
        || limits.max_pages_per_server == 0
        || limits.max_pages_per_server > 256
    {
        return Err(ConfigError::Validation(
            "External MCP catalog limits must be positive and within their hard ceilings".into(),
        ));
    }
    // A catalog that legally cannot be buffered in one response is refused here rather than
    // becoming a server that silently fails to resolve at admission.
    if crate::tools::external_mcp::response_byte_cap(limits).is_none() {
        return Err(ConfigError::Validation(
            "External MCP catalog limits describe a page larger than one response can hold; lower max_tools_per_server or max_tool_schema_bytes".into(),
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
    use crate::config::{LlmInstanceConfig, TtsInstanceConfig};
    let registry = crate::providers::compiled_provider_registry();
    for (id, instance) in &config.providers.vad.instances {
        validate_instance_id(id)?;
        registry.vad_factory(instance.adapter()).map_err(|_| {
            ConfigError::Validation(format!(
                "VAD instance `{id}` uses adapter `{}` which is not compiled into this binary",
                instance.adapter()
            ))
        })?;
        if let crate::config::VadInstanceConfig::SileroOnnx(vad) = instance
            && (vad.min_speech_ms == 0
                || vad.end_silence_ms == 0
                || vad.pre_roll_ms > config.audio.max_utterance_ms
                || vad.num_threads <= 0
                || !vad.speech_threshold.is_finite()
                || !vad.exit_threshold.is_finite()
                || !(0.0..=1.0).contains(&vad.exit_threshold)
                || vad.exit_threshold >= vad.speech_threshold
                || vad.speech_threshold > 1.0
                || vad.model.trim().is_empty())
        {
            return Err(ConfigError::Validation(format!(
                "VAD instance `{id}` has invalid thresholds, durations, or model identity"
            )));
        }
    }
    if config
        .runtime
        .onnx
        .threads
        .iter()
        .any(|(adapter, threads)| {
            crate::providers::local_model_identity(adapter).is_none()
                || !(1..=128).contains(threads)
        })
        || config.runtime.chillaudio.ws_url.scheme() != "wss"
        || config.runtime.chillaudio.ws_url.host_str().is_none()
        || !(1..=120_000).contains(&config.runtime.chillaudio.timeout_ms)
    {
        return Err(ConfigError::Validation(
            "invalid server-owned provider runtime configuration".into(),
        ));
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
                if asr.num_threads <= 0 || asr.model.trim().is_empty() =>
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
            #[cfg(feature = "qualification-providers")]
            crate::config::AsrInstanceConfig::QualificationAsr(_) => {}
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
        #[allow(irrefutable_let_patterns)] // Qualification builds add another LLM variant.
        let LlmInstanceConfig::Openai(openai) = instance else {
            #[cfg(feature = "qualification-providers")]
            {
                continue;
            }
            #[cfg(not(feature = "qualification-providers"))]
            unreachable!();
        };
        let http_allowed = openai.base_url.scheme() == "http"
            && match openai.base_url.host_str() {
                Some("localhost") => true,
                Some(host) => host.parse::<IpAddr>().is_ok_and(|ip| match ip {
                    IpAddr::V4(ip) => ip.is_private() || ip.is_link_local() || ip.is_loopback(),
                    IpAddr::V6(ip) => {
                        ip.is_unique_local() || ip.is_unicast_link_local() || ip.is_loopback()
                    }
                }),
                None => false,
            };
        let scheme_allowed = openai.base_url.scheme() == "https" || http_allowed;
        if !scheme_allowed
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
            TtsInstanceConfig::KokoroViOnnx(tts)
                if tts.model.trim().is_empty()
                    || tts.voice.trim().is_empty()
                    || tts.language != "vi-VN"
                    || tts.num_threads <= 0
                    || !(50..=200).contains(&tts.speed_percent) =>
            {
                return Err(ConfigError::Validation(format!(
                    "Kokoro Vietnamese instance `{id}` has invalid model, voice, language, threads, or speed"
                )));
            }
            #[cfg(feature = "qualification-providers")]
            TtsInstanceConfig::QualificationTts(_) => {}
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
    let binding = config.provider_defaults.vision.as_deref().ok_or_else(|| {
        ConfigError::Validation("vision.enabled requires a default Vision provider binding".into())
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
    if config.deployment.profile.is_empty() {
        return Err(ConfigError::Validation(
            "deployment profile must be set".into(),
        ));
    }
    if config.runtime.onnx.library.as_os_str().is_empty() {
        return Err(ConfigError::Validation(
            "runtime.onnx.library must be set".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A configuration that parses, so a test can be about the database section and nothing else.
    fn config(extra: &str) -> AppConfig {
        toml::from_str(&format!(
            r#"
[server]
bind = "127.0.0.1:0"
public_ws_url = "ws://127.0.0.1:0/voice/v1/"

[provider_defaults]
vad = "vad"
asr = "asr"
llm = "llm"
tts = "tts"

[database]
url = "sqlite://data/voice-agent.db"
{extra}
"#
        ))
        .expect("the minimal configuration parses")
    }

    #[test]
    fn capture_and_retention_bounds_are_checked_before_a_listener_binds() {
        assert!(validate_database(&config("")).is_ok());

        assert!(
            validate_database(&config(
                r#"
[database.history]
enabled = true
"#
            ))
            .is_ok(),
            "the opt-in is accepted with the documented defaults"
        );
        assert!(
            validate_database(&config(
                r#"
[database.history]
retention_days = 0
"#
            ))
            .is_err(),
            "an archive with no retention window is not a retention policy"
        );
        assert!(
            validate_database(&config(
                r#"
[database.history]
retention_days = 366
"#
            ))
            .is_err()
        );
        assert!(
            validate_database(&config(
                r#"
[database.history]
queue_capacity = 0
"#
            ))
            .is_err(),
            "a zero-capacity hand-off could only ever drop every record"
        );
    }

    #[test]
    fn retention_is_bounded_whether_or_not_capture_is_on() {
        let mut capture_off = config(
            r#"
[database.history]
enabled = false
retention_days = 0
"#,
        );
        assert!(
            validate_database(&capture_off).is_err(),
            "turning capture off must not make retention unbounded either"
        );
        capture_off.database.history.retention_days = 30;
        assert!(validate_database(&capture_off).is_ok());
    }

    #[test]
    fn speaker_recognition_bounds_are_enforced() {
        assert!(validate_speaker_recognition(&config("")).is_ok());
        assert!(
            validate_speaker_recognition(&config(
                r#"
[speaker_recognition]
max_speakers = 0
"#
            ))
            .is_err()
        );
        assert!(
            validate_speaker_recognition(&config(
                r#"
[speaker_recognition.enrollment]
min_samples = 5
max_samples = 3
"#
            ))
            .is_err()
        );
        assert!(
            validate_speaker_recognition(&config(
                r#"
[speaker_recognition.enrollment]
min_clip_ms = 10_000
max_clip_ms = 5_000
"#
            ))
            .is_err()
        );
        assert!(
            validate_speaker_recognition(&config(
                r#"
[speaker_recognition.enrollment]
ttl_ms = 1_000
"#
            ))
            .is_err()
        );
    }
}
