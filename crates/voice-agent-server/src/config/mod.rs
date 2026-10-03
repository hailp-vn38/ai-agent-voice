use serde::Deserialize;
use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
};
use url::Url;

mod validation;

pub use validation::{BenchmarkTarget, ConfigError};

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
    pub provider_defaults: ProviderDefaultsConfig,
    #[serde(default)]
    pub providers: ProvidersConfig,
    #[serde(default)]
    pub workers: WorkersConfig,
    #[serde(default)]
    pub deployment: DeploymentConfig,
    #[serde(default)]
    pub runtime: RuntimeConfig,
    #[serde(default)]
    pub provider_runtime: Option<ProviderRuntimeConfig>,
    #[serde(default)]
    pub llm: LlmConfig,
    #[serde(default)]
    pub tts: TtsConfig,
    #[serde(default)]
    pub speech_output: SpeechOutputConfig,
    #[serde(default)]
    pub barge_in: BargeInConfig,
    #[serde(default)]
    pub mcp: McpConfig,
    #[serde(default)]
    pub vision: VisionConfig,
    #[serde(default)]
    pub database: DatabaseConfig,
    #[serde(default)]
    pub api: AdminApiConfig,
    #[serde(default)]
    pub shutdown: ShutdownConfig,
    #[serde(default)]
    pub agent: Option<AgentConfig>,
    #[serde(skip)]
    pub effective_agent: EffectiveAgentConfig,
}

pub const DEFAULT_AGENT_NAME: &str = "Mây";
pub const DEFAULT_AGENT_LANGUAGE: &str = "vi-VN";
pub const DEFAULT_AGENT_PERSONA: &str = "\
Bạn là Mây, một trợ lý giọng nói tiếng Việt.
Bạn trả lời tự nhiên, thân thiện và súc tích.
Ưu tiên nội dung dễ nghe qua loa.";
pub const DEFAULT_PROMPT_TEMPLATE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../prompts/voice-assistant.txt"
));
const MAX_PERSONA_BYTES: usize = 16 * 1024;
const MAX_PROMPT_TEMPLATE_BYTES: usize = 64 * 1024;
/// Hard bound for any system prompt a session may compose, whether it came from the deployment
/// template or from a stored Template row.
pub const MAX_RENDERED_SYSTEM_PROMPT_BYTES: usize = 96 * 1024;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentConfig {
    pub name: Option<String>,
    pub language: Option<String>,
    pub prompt_template: Option<PathBuf>,
    pub persona: Option<String>,
    #[serde(default)]
    pub providers: AgentProviderBindings,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderDefaultsConfig {
    pub vad: String,
    pub asr: String,
    pub llm: String,
    pub tts: String,
    #[serde(default)]
    pub vision: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentProviderBindings {
    pub vad: Option<String>,
    pub asr: Option<String>,
    pub llm: Option<String>,
    pub tts: Option<String>,
    pub vision: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectiveProviderBindings {
    pub vad: String,
    pub asr: String,
    pub llm: String,
    pub tts: String,
    pub vision: Option<String>,
}

#[derive(Clone, Debug)]
pub struct EffectiveAgentConfig {
    pub name: String,
    pub language: String,
    pub persona: String,
    pub prompt_template: String,
    pub providers: EffectiveProviderBindings,
}

impl EffectiveAgentConfig {
    fn with_provider_defaults(provider_defaults: &ProviderDefaultsConfig) -> Self {
        Self {
            name: DEFAULT_AGENT_NAME.into(),
            language: DEFAULT_AGENT_LANGUAGE.into(),
            persona: DEFAULT_AGENT_PERSONA.into(),
            prompt_template: DEFAULT_PROMPT_TEMPLATE.into(),
            providers: EffectiveProviderBindings {
                vad: provider_defaults.vad.clone(),
                asr: provider_defaults.asr.clone(),
                llm: provider_defaults.llm.clone(),
                tts: provider_defaults.tts.clone(),
                vision: provider_defaults.vision.clone(),
            },
        }
    }
}

impl Default for EffectiveAgentConfig {
    fn default() -> Self {
        Self {
            name: DEFAULT_AGENT_NAME.into(),
            language: DEFAULT_AGENT_LANGUAGE.into(),
            persona: DEFAULT_AGENT_PERSONA.into(),
            prompt_template: DEFAULT_PROMPT_TEMPLATE.into(),
            providers: EffectiveProviderBindings {
                vad: String::new(),
                asr: String::new(),
                llm: String::new(),
                tts: String::new(),
                vision: None,
            },
        }
    }
}

impl AppConfig {
    pub fn effective_agent(&self) -> &EffectiveAgentConfig {
        &self.effective_agent
    }

    pub(crate) fn resolve_agent(&mut self, config_path: &Path) -> Result<(), ConfigError> {
        let mut effective = EffectiveAgentConfig::with_provider_defaults(&self.provider_defaults);
        let Some(overrides) = &mut self.agent else {
            self.effective_agent = effective;
            return Ok(());
        };
        effective.providers = EffectiveProviderBindings {
            vad: overrides
                .providers
                .vad
                .clone()
                .unwrap_or(effective.providers.vad),
            asr: overrides
                .providers
                .asr
                .clone()
                .unwrap_or(effective.providers.asr),
            llm: overrides
                .providers
                .llm
                .clone()
                .unwrap_or(effective.providers.llm),
            tts: overrides
                .providers
                .tts
                .clone()
                .unwrap_or(effective.providers.tts),
            vision: overrides
                .providers
                .vision
                .clone()
                .or(effective.providers.vision),
        };
        if let Some(name) = &overrides.name {
            if name.trim().is_empty() {
                return Err(ConfigError::Validation(
                    "agent.name must not be empty when declared".into(),
                ));
            }
            effective.name = name.clone();
        }
        if let Some(language) = &overrides.language {
            if language.trim().is_empty() {
                return Err(ConfigError::Validation(
                    "agent.language must not be empty when declared".into(),
                ));
            }
            effective.language = language.clone();
        }
        if let Some(persona) = &overrides.persona {
            if persona.trim().is_empty() {
                return Err(ConfigError::Validation(
                    "agent.persona must not be empty when declared".into(),
                ));
            }
            effective.persona = persona.clone();
        }
        if effective.persona.len() > MAX_PERSONA_BYTES {
            return Err(ConfigError::Validation(
                "agent.persona exceeds 16 KiB".into(),
            ));
        }
        if let Some(path) = &mut overrides.prompt_template {
            if path.as_os_str().is_empty() {
                return Err(ConfigError::Validation(
                    "agent.prompt_template must not be empty when declared".into(),
                ));
            }
            if path.is_relative() {
                *path = config_path
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join(&*path);
            }
            if !std::fs::metadata(&*path)
                .map_err(ConfigError::Read)?
                .is_file()
            {
                return Err(ConfigError::Validation(
                    "agent.prompt_template must be a regular file".into(),
                ));
            }
            effective.prompt_template =
                std::fs::read_to_string(&*path).map_err(ConfigError::Read)?;
        }
        if effective.prompt_template.len() > MAX_PROMPT_TEMPLATE_BYTES {
            return Err(ConfigError::Validation(
                "agent.prompt_template exceeds 64 KiB".into(),
            ));
        }
        validate_prompt_template(&effective.prompt_template)?;
        let rendered_len = effective
            .prompt_template
            .replace("{{agent_name}}", &effective.name)
            .replace("{{persona}}", &effective.persona)
            .replace("{{language}}", &effective.language)
            .len();
        if rendered_len > MAX_RENDERED_SYSTEM_PROMPT_BYTES {
            return Err(ConfigError::Validation(
                "rendered agent system prompt exceeds 96 KiB".into(),
            ));
        }
        self.effective_agent = effective;
        Ok(())
    }
}

fn validate_prompt_template(template: &str) -> Result<(), ConfigError> {
    if template.contains("{%") {
        return Err(ConfigError::Validation(
            "agent.prompt_template has invalid placeholder grammar".into(),
        ));
    }
    let mut cursor = 0;
    let mut persona_seen = false;
    while let Some(relative) = template[cursor..].find("{{") {
        let start = cursor + relative;
        if template[cursor..start].contains("}}") {
            return Err(ConfigError::Validation(
                "agent.prompt_template has invalid placeholder grammar".into(),
            ));
        }
        let token_start = start + 2;
        let Some(end_relative) = template[token_start..].find("}}") else {
            return Err(ConfigError::Validation(
                "agent.prompt_template has invalid placeholder grammar".into(),
            ));
        };
        let end = token_start + end_relative;
        match &template[token_start..end] {
            "agent_name" | "language" => {}
            "persona" => persona_seen = true,
            _ => {
                return Err(ConfigError::Validation(
                    "agent.prompt_template has invalid placeholder grammar".into(),
                ));
            }
        }
        cursor = end + 2;
    }
    if template[cursor..].contains("}}") {
        return Err(ConfigError::Validation(
            "agent.prompt_template has invalid placeholder grammar".into(),
        ));
    }
    if !persona_seen {
        return Err(ConfigError::Validation(
            "agent.prompt_template must contain {{persona}}".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod database_history_config_tests {
    use super::{DatabaseConfig, DatabaseHistoryConfig};

    fn same(left: &DatabaseHistoryConfig, right: &DatabaseHistoryConfig) -> bool {
        left.enabled == right.enabled
            && left.retention_days == right.retention_days
            && left.queue_capacity == right.queue_capacity
    }

    /// `DatabaseHistoryConfig::default` restates the `#[serde(default = …)]` values on purpose, for
    /// the same reason `LlmToolsConfig::default` does: a section that deserialized to zeroes is a
    /// configuration that fails its own validation.
    #[test]
    fn the_default_history_configuration_is_the_documented_one() {
        let default = DatabaseHistoryConfig::default();
        assert!(!default.enabled, "capture is opt-in");
        assert_eq!(default.retention_days, 30);
        assert_eq!(default.queue_capacity, 256);
        assert!(same(&default, &DatabaseConfig::default().history));

        #[derive(serde::Deserialize)]
        struct Wrapper {
            database: super::DatabaseConfig,
        }
        let parsed: Wrapper = toml::from_str(
            r#"
            [database]
            enabled = true
            url = "sqlite://data/voice-agent.db"
            "#,
        )
        .expect("a configuration with no history section is still a configuration");
        assert!(same(
            &parsed.database.history,
            &DatabaseHistoryConfig::default()
        ));
    }
}

#[cfg(test)]
mod agent_template_tests {
    use super::validate_prompt_template;

    #[test]
    fn accepts_only_the_three_literal_placeholders() {
        assert!(validate_prompt_template("{{persona}} {{agent_name}} {{language}}").is_ok());
        for invalid in [
            "{{persona}} {{ persona }}",
            "{{persona}} {{foo}}",
            "{{persona}} {{persona.name}}",
            "{{{persona}}}",
            "{{persona}} {% x %}",
        ] {
            assert!(validate_prompt_template(invalid).is_err(), "{invalid}");
        }
    }
}

#[cfg(test)]
mod external_mcp_config_tests {
    use super::{ExternalMcpConfig, ExternalMcpLimitsConfig, McpConfig};
    use toml;

    /// The `Default` impls below restate the `#[serde(default = …)]` values on purpose: an
    /// `ExternalMcpConfig` that deserialized to zeroes would be a configuration that fails its own
    /// validation, and `McpConfig::default()` is what a deployment that names no `[mcp.external]`
    /// section actually runs with.  This test is what keeps the two spellings in step.
    #[test]
    fn the_default_external_mcp_configuration_is_the_documented_one() {
        let default = McpConfig::default().external;
        assert_eq!(default.per_server_resolution_timeout_ms, 3_000);
        assert_eq!(default.overall_resolution_budget_ms, 5_000);
        assert_eq!(default.max_concurrent_calls_per_server, 16);
        assert_eq!(default.limits, ExternalMcpLimitsConfig::default());
        assert_eq!(default.limits.max_tools_per_server, 128);
        assert_eq!(default.limits.max_tools_per_session, 512);
        assert_eq!(default.limits.max_tool_schema_bytes, 16_384);
        assert_eq!(default.limits.max_tool_description_bytes, 4_096);
        assert_eq!(default.limits.max_external_tool_result_bytes, 16_384);
        assert_eq!(default.limits.max_pages_per_server, 32);
        assert_eq!(default, ExternalMcpConfig::default());
    }

    /// An omitted `[mcp.external]` section must produce exactly the default, not a partial one.
    #[test]
    fn an_omitted_section_deserializes_to_the_documented_defaults() {
        #[derive(serde::Deserialize)]
        struct Wrapper {
            mcp: McpConfig,
        }
        let parsed: Wrapper = toml::from_str(
            r#"
            [mcp]
            enabled = true
            "#,
        )
        .expect("a configuration with no external section is still a configuration");
        assert_eq!(parsed.mcp.external, ExternalMcpConfig::default());
    }
}

#[cfg(test)]
mod tool_round_config_tests {
    use super::{LlmConfig, LlmToolsConfig, validation::validate_tool_rounds};

    /// `LlmToolsConfig::default` restates the `#[serde(default = …)]` values on purpose, for the
    /// same reason `ExternalMcpConfig` does: a section that deserialized to zeroes is a
    /// configuration that fails its own validation.
    #[test]
    fn the_default_tool_round_configuration_is_the_documented_one() {
        let default = LlmToolsConfig::default();
        assert_eq!(default.max_calls_per_round, 8);
        assert_eq!(default.max_rounds_per_turn, 4);
        assert_eq!(default.execution_budget_ms, 30_000);
        assert_eq!(default, LlmConfig::default().tools);

        #[derive(serde::Deserialize)]
        struct Wrapper {
            llm: LlmConfig,
        }
        let parsed: Wrapper = toml::from_str(
            r#"
            [llm]
            max_history_messages = 20
            "#,
        )
        .expect("a configuration with no tools section is still a configuration");
        assert_eq!(parsed.llm.tools, LlmToolsConfig::default());
    }

    /// A zero cap is refused as loudly as an oversized one: `0` would mean the executor silently
    /// refuses every tool call instead of the deployment failing at startup.
    #[test]
    fn every_tool_round_cap_refuses_zero_and_its_hard_ceiling() {
        let base = LlmToolsConfig::default();
        for (calls, rounds, budget) in [
            (0, 4, 30_000),
            (33, 4, 30_000),
            (8, 0, 30_000),
            (8, 9, 30_000),
            (8, 4, 0),
            (8, 4, 120_001),
        ] {
            let candidate = LlmToolsConfig {
                max_calls_per_round: calls,
                max_rounds_per_turn: rounds,
                execution_budget_ms: budget,
            };
            assert!(
                validate_tool_rounds(&candidate).is_err(),
                "calls={calls} rounds={rounds} budget_ms={budget} must fail startup"
            );
        }
        assert!(
            validate_tool_rounds(&base).is_ok(),
            "the documented defaults are a valid configuration"
        );
    }
}

pub(crate) mod defaults;
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
    #[serde(default)]
    pub kokoro_vi: KokoroViRuntimeConfig,
}

/// Deployment-owned executable for Vietnamese grapheme-to-phoneme conversion.
/// It is outside mutable provider configuration because it is an execution boundary.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KokoroViRuntimeConfig {
    #[serde(default = "default_kokoro_vi_g2p_executable")]
    pub g2p_executable: std::path::PathBuf,
}

impl Default for KokoroViRuntimeConfig {
    fn default() -> Self {
        Self {
            g2p_executable: default_kokoro_vi_g2p_executable(),
        }
    }
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
#[serde(deny_unknown_fields)]
pub struct LlmConfig {
    #[serde(default = "default_max_history_messages")]
    pub max_history_messages: usize,
    #[serde(default = "default_prompt_budget_tokens")]
    pub prompt_budget_tokens: usize,
    #[serde(default = "default_max_tool_result_chars")]
    pub max_tool_result_chars: usize,
    #[serde(default)]
    pub tools: LlmToolsConfig,
}

/// The Tool-round Executor's three policy caps.
///
/// These are policy, not queue capacity: nothing here pre-allocates.  A cap is checked before the
/// work it bounds, and exceeding one terminalizes the Conversational Turn rather than producing a
/// synthetic ToolResult, because a cap is never a completed ToolCall's outcome.
///
/// Every bound is checked once, here at the configuration layer, before a listener binds.  The
/// executor takes the validated values and never re-derives them.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LlmToolsConfig {
    /// How many ToolCalls one LLM round may contain.  The whole round is validated before call
    /// one, so exceeding this executes no call at all.
    #[serde(default = "default_max_calls_per_round")]
    pub max_calls_per_round: usize,
    /// How many tool rounds one Conversational Turn may continue past its first.  Checked before
    /// the next round's request is sent, so exceeding it starts no further call.
    #[serde(default = "default_max_rounds_per_turn")]
    pub max_rounds_per_turn: usize,
    /// The Tool Execution Budget: what one Conversational Turn may spend on tool work in total,
    /// starting at its first ToolCall.  Each call is bounded by what is left of it, so waiting for
    /// an outbound permit is spent out of the same budget as the request.
    #[serde(default = "default_tool_execution_budget_ms")]
    pub execution_budget_ms: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpConfig {
    #[serde(default = "default_mcp_enabled")]
    pub enabled: bool,
    #[serde(default = "default_mcp_call_timeout_ms")]
    pub call_timeout_ms: u64,
    #[serde(default = "default_mcp_discovery_timeout_ms")]
    pub discovery_timeout_ms: u64,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub result_delivery: McpResultDelivery,
    #[serde(default)]
    pub tool_policy: Vec<McpToolPolicy>,
    #[serde(default)]
    pub external: ExternalMcpConfig,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalMcpConfig {
    /// Budget for one server's whole `initialize` + paginated `tools/list` walk.  A server that
    /// cannot finish inside it is excluded, never truncated.
    #[serde(default = "default_external_per_server_resolution_timeout_ms")]
    pub per_server_resolution_timeout_ms: u64,
    /// Budget for the entire admission snapshot across every bound server.
    #[serde(default = "default_external_overall_resolution_budget_ms")]
    pub overall_resolution_budget_ms: u64,
    /// Process-global concurrency bound per immutable MCP server identity, shared by every session.
    #[serde(default = "default_external_max_concurrent_calls_per_server")]
    pub max_concurrent_calls_per_server: u32,
    #[serde(default)]
    pub limits: ExternalMcpLimitsConfig,
    #[serde(default)]
    pub network: ExternalMcpNetworkConfig,
}

impl Default for ExternalMcpConfig {
    fn default() -> Self {
        Self {
            per_server_resolution_timeout_ms: default_external_per_server_resolution_timeout_ms(),
            overall_resolution_budget_ms: default_external_overall_resolution_budget_ms(),
            max_concurrent_calls_per_server: default_external_max_concurrent_calls_per_server(),
            limits: ExternalMcpLimitsConfig::default(),
            network: ExternalMcpNetworkConfig::default(),
        }
    }
}

/// Caps applied at the raw, untrusted catalog boundary before anything is converted for the LLM.
/// Every cap rejects the whole affected catalog; none of them truncates.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalMcpLimitsConfig {
    #[serde(default = "default_external_max_tools_per_server")]
    pub max_tools_per_server: usize,
    #[serde(default = "default_external_max_tools_per_session")]
    pub max_tools_per_session: usize,
    #[serde(default = "default_external_max_tool_schema_bytes")]
    pub max_tool_schema_bytes: usize,
    #[serde(default = "default_external_max_tool_description_bytes")]
    pub max_tool_description_bytes: usize,
    #[serde(default = "default_external_max_tool_result_bytes")]
    pub max_external_tool_result_bytes: usize,
    #[serde(default = "default_external_max_pages_per_server")]
    pub max_pages_per_server: usize,
}

impl Default for ExternalMcpLimitsConfig {
    fn default() -> Self {
        Self {
            max_tools_per_server: default_external_max_tools_per_server(),
            max_tools_per_session: default_external_max_tools_per_session(),
            max_tool_schema_bytes: default_external_max_tool_schema_bytes(),
            max_tool_description_bytes: default_external_max_tool_description_bytes(),
            max_external_tool_result_bytes: default_external_max_tool_result_bytes(),
            max_pages_per_server: default_external_max_pages_per_server(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalMcpNetworkConfig {
    #[serde(default)]
    pub allow_http_lan: bool,
    #[serde(default)]
    pub allowed_hosts: Vec<String>,
    #[serde(default)]
    pub allowed_cidrs: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum McpResultDelivery {
    #[default]
    LlmThenTts,
    DirectTts,
    Silent,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpToolPolicy {
    pub name: String,
    pub result_delivery: McpResultDelivery,
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            enabled: default_mcp_enabled(),
            call_timeout_ms: default_mcp_call_timeout_ms(),
            discovery_timeout_ms: default_mcp_discovery_timeout_ms(),
            allowed_tools: Vec::new(),
            result_delivery: McpResultDelivery::default(),
            tool_policy: Vec::new(),
            external: ExternalMcpConfig::default(),
        }
    }
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
            prompt_budget_tokens: default_prompt_budget_tokens(),
            max_tool_result_chars: default_max_tool_result_chars(),
            tools: LlmToolsConfig::default(),
        }
    }
}

impl Default for LlmToolsConfig {
    fn default() -> Self {
        Self {
            max_calls_per_round: default_max_calls_per_round(),
            max_rounds_per_turn: default_max_rounds_per_turn(),
            execution_budget_ms: default_tool_execution_budget_ms(),
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

/// Optional local SQLite control plane. A single process owns each configured path.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatabaseConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_database_url")]
    pub url: String,
    #[serde(default = "default_database_max_connections")]
    pub max_connections: u32,
    #[serde(default = "default_database_busy_timeout_ms")]
    pub busy_timeout_ms: u64,
    #[serde(default = "default_true")]
    pub migrate_on_start: bool,
    #[serde(default)]
    pub devices: DatabaseDevicesConfig,
    #[serde(default)]
    pub history: DatabaseHistoryConfig,
}

/// Explicit controls for database-backed Voice Protocol Client admission.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatabaseDevicesConfig {
    #[serde(default)]
    pub admission_enabled: bool,
    #[serde(default)]
    pub auto_register: bool,
    #[serde(default)]
    pub auto_register_agent_key: String,
    #[serde(default)]
    pub enrollment: EnrollmentConfig,
}

/// Optional control-plane enrollment for an unknown Voice Protocol Client.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnrollmentConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_enrollment_code_ttl_seconds")]
    pub code_ttl_seconds: u64,
    #[serde(default = "default_enrollment_retention_seconds")]
    pub retention_seconds: u64,
    #[serde(default = "default_enrollment_cleanup_interval_seconds")]
    pub cleanup_interval_seconds: u64,
    #[serde(default = "default_enrollment_max_pending")]
    pub max_pending: u32,
}

impl Default for EnrollmentConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            code_ttl_seconds: default_enrollment_code_ttl_seconds(),
            retention_seconds: default_enrollment_retention_seconds(),
            cleanup_interval_seconds: default_enrollment_cleanup_interval_seconds(),
            max_pending: default_enrollment_max_pending(),
        }
    }
}

impl Default for DatabaseDevicesConfig {
    fn default() -> Self {
        Self {
            admission_enabled: false,
            auto_register: false,
            auto_register_agent_key: String::new(),
            enrollment: EnrollmentConfig::default(),
        }
    }
}

/// Optional Persistent Transcript capture and the retention of the archive it writes.
///
/// Capture and retention are separate policies.  `enabled` decides whether a Voice Session
/// enqueues anything at all; `retention_days` decides how long whatever is already archived
/// survives, so switching capture off never turns existing data into unbounded retention.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatabaseHistoryConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_history_retention_days")]
    pub retention_days: u32,
    /// Records the archival writer may hold before a new one is dropped.  Bounded so a slow
    /// database can never turn into an unbounded in-process backlog.
    #[serde(default = "default_history_queue_capacity")]
    pub queue_capacity: usize,
}

impl Default for DatabaseHistoryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            retention_days: default_history_retention_days(),
            queue_capacity: default_history_queue_capacity(),
        }
    }
}

/// Optional, separately authenticated administrative control plane.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdminApiConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub admin_token: String,
    #[serde(default)]
    pub provider_tests: ProviderTestsConfig,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderTestsConfig {
    #[serde(default = "default_provider_test_concurrency")]
    pub max_concurrency: usize,
    #[serde(default = "default_provider_test_timeout_ms")]
    pub timeout_ms: u64,
}
impl Default for ProviderTestsConfig {
    fn default() -> Self {
        Self {
            max_concurrency: default_provider_test_concurrency(),
            timeout_ms: default_provider_test_timeout_ms(),
        }
    }
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            url: default_database_url(),
            max_connections: default_database_max_connections(),
            busy_timeout_ms: default_database_busy_timeout_ms(),
            migrate_on_start: true,
            devices: DatabaseDevicesConfig::default(),
            history: DatabaseHistoryConfig::default(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShutdownConfig {
    #[serde(default = "default_shutdown_grace_ms")]
    pub grace_ms: u64,
}

impl Default for ShutdownConfig {
    fn default() -> Self {
        Self {
            grace_ms: default_shutdown_grace_ms(),
        }
    }
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
    #[serde(default = "default_vision_concurrency")]
    pub vision_concurrency: usize,
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
            vision_concurrency: default_vision_concurrency(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VisionConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub public_url: Option<Url>,
    #[serde(default = "default_vision_max_image_bytes")]
    pub max_image_bytes: usize,
    #[serde(default = "default_vision_max_question_bytes")]
    pub max_question_bytes: usize,
    #[serde(default)]
    pub advertise_via_mcp: bool,
}

impl Default for VisionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            public_url: None,
            max_image_bytes: default_vision_max_image_bytes(),
            max_question_bytes: default_vision_max_question_bytes(),
            advertise_via_mcp: false,
        }
    }
}

/// Explicit deployment measurements: no fabricated default for native resident memory.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderRuntimeConfig {
    #[serde(flatten)]
    pub limits: crate::services::provider_runtime::RuntimeLimits,
    pub estimated_peak_bytes: std::collections::HashMap<String, u64>,
    pub measured_manifest_sha256: String,
    #[serde(default = "default_provider_startup_timeout_ms")]
    pub startup_timeout_ms: u64,
}

fn default_provider_startup_timeout_ms() -> u64 {
    60_000
}
