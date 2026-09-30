//! Immutable, UI-neutral metadata for provider adapters compiled into the Admin control plane.
//!
//! Descriptors describe adapter capabilities; they never describe a persisted provider instance or
//! a loaded runtime.  Bootstrap inspection below is deliberately metadata-only, so it can run
//! before an instance exists without resolving a secret or loading a model.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderType {
    Vad,
    Asr,
    Llm,
    Tts,
}

#[derive(Serialize)]
pub struct ProviderDescriptor {
    pub adapter: &'static str,
    #[serde(rename = "type")]
    pub provider_type: ProviderType,
    pub display_name: &'static str,
    pub description: &'static str,
    pub config_schema: ProviderConfigSchema,
    pub capabilities: ProviderCapabilities,
    pub discovery: CapabilityDiscoveryMode,
}

#[derive(Serialize)]
pub struct ProviderConfigSchema {
    pub fields: &'static [ProviderConfigField],
}

#[derive(Serialize)]
pub struct ProviderConfigField {
    pub key: &'static str,
    pub label: &'static str,
    #[serde(rename = "type")]
    pub field_type: ConfigFieldType,
    pub required: bool,
    pub nullable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enum_source: Option<CapabilitySource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enum_values: Option<&'static [&'static str]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minimum: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maximum: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_length: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<&'static str>,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigFieldType {
    String,
    Integer,
    Boolean,
    Select,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilitySource {
    Models,
    Voices,
    Languages,
}

#[derive(Serialize)]
pub struct ProviderCapabilities {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub models: Option<&'static [ModelOption]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voices: Option<&'static [VoiceOption]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub languages: Option<&'static [LanguageOption]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub streaming: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offline: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calling: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vision: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_sample_rates: Option<&'static [u32]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channels: Option<&'static [u8]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_output_sample_rates: Option<&'static [u32]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voice_delivery_sample_rates: Option<&'static [u32]>,
}

#[derive(Serialize)]
pub struct ModelOption {
    pub id: &'static str,
    pub name: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<&'static str>,
}

#[derive(Serialize)]
pub struct LanguageOption {
    pub id: &'static str,
    pub name: &'static str,
}

#[derive(Serialize)]
pub struct VoiceOption {
    pub id: &'static str,
    pub name: &'static str,
    pub languages: &'static [&'static str],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<&'static str>,
}

#[derive(Serialize)]
pub struct CapabilityDiscoveryMode {
    pub models: DiscoverySource,
    pub voices: DiscoverySource,
    pub languages: DiscoverySource,
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoverySource {
    Unsupported,
    Static,
    Bootstrap,
    Runtime,
    BootstrapAndRuntime,
    Remote,
}

#[derive(Serialize)]
pub struct AdapterSummary {
    pub adapter: &'static str,
    #[serde(rename = "type")]
    pub provider_type: ProviderType,
    pub display_name: &'static str,
}

impl From<&'static ProviderDescriptor> for AdapterSummary {
    fn from(descriptor: &'static ProviderDescriptor) -> Self {
        Self {
            adapter: descriptor.adapter,
            provider_type: descriptor.provider_type,
            display_name: descriptor.display_name,
        }
    }
}

#[derive(Serialize)]
pub struct DiscoveredCapabilities {
    pub models: &'static [ModelOption],
    pub voices: &'static [VoiceOption],
    pub languages: &'static [LanguageOption],
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderInspectError {
    #[error("invalid bootstrap selection")]
    InvalidSelection,
    #[error("bootstrap discovery is not supported")]
    Unsupported,
}

pub struct ProviderAdapterRegistry {
    descriptors: &'static [ProviderDescriptor],
}

impl ProviderAdapterRegistry {
    pub fn list(
        &self,
        provider_type: Option<ProviderType>,
    ) -> impl Iterator<Item = &'static ProviderDescriptor> {
        self.descriptors.iter().filter(move |descriptor| {
            provider_type.is_none_or(|kind| descriptor.provider_type == kind)
        })
    }

    pub fn get(&self, adapter: &str) -> Option<&'static ProviderDescriptor> {
        self.descriptors
            .iter()
            .find(|descriptor| descriptor.adapter == adapter)
    }

    pub fn supports(&self, adapter: &str, provider_type: &str) -> bool {
        self.get(adapter)
            .is_some_and(|descriptor| descriptor.provider_type.as_str() == provider_type)
    }

    pub fn discover(
        &self,
        adapter: &str,
        selection: &Value,
    ) -> Result<DiscoveredCapabilities, ProviderInspectError> {
        match adapter {
            "zerotts_onnx" => zerotts_bootstrap(selection),
            "gipformer_sherpa_offline" => gipformer_bootstrap(selection),
            _ => Err(ProviderInspectError::Unsupported),
        }
    }
}

impl ProviderType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Vad => "vad",
            Self::Asr => "asr",
            Self::Llm => "llm",
            Self::Tts => "tts",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "vad" => Some(Self::Vad),
            "asr" => Some(Self::Asr),
            "llm" => Some(Self::Llm),
            "tts" => Some(Self::Tts),
            _ => None,
        }
    }
}

const MODELS_ZEROTTS: &[ModelOption] = &[ModelOption {
    id: "zerotts_default",
    name: "ZeroTTS Default",
    description: None,
}];
const MODELS_ZIPFORMER: &[ModelOption] = &[ModelOption {
    id: "zipformer_vi_streaming",
    name: "Zipformer Vietnamese Streaming",
    description: None,
}];
const MODELS_GIPFORMER: &[ModelOption] = &[ModelOption {
    id: "gipformer15_vi_int8",
    name: "Gipformer 1.5 Vietnamese INT8",
    description: None,
}];
const MODELS_SILERO: &[ModelOption] = &[ModelOption {
    id: "silero_vad_v5",
    name: "Silero VAD v5",
    description: None,
}];
const VIETNAMESE: &[LanguageOption] = &[LanguageOption {
    id: "vi-VN",
    name: "Vietnamese",
}];
const MAICHI: &[VoiceOption] = &[VoiceOption {
    id: "maichi",
    name: "Mai Chi",
    languages: &["vi-VN"],
    model: Some("zerotts_default"),
}];

const SILERO_FIELDS: &[ProviderConfigField] = &[
    field(
        "model",
        "Model",
        ConfigFieldType::Select,
        true,
        Some(CapabilitySource::Models),
        None,
        None,
        Some(128),
    ),
    field(
        "num_threads",
        "Threads",
        ConfigFieldType::Integer,
        true,
        None,
        Some(1),
        Some(128),
        None,
    ),
];
const ZIPFORMER_FIELDS: &[ProviderConfigField] = &[
    field(
        "model",
        "Model",
        ConfigFieldType::Select,
        true,
        Some(CapabilitySource::Models),
        None,
        None,
        Some(128),
    ),
    field(
        "num_threads",
        "Threads",
        ConfigFieldType::Integer,
        true,
        None,
        Some(1),
        Some(128),
        None,
    ),
    field(
        "decoding_method",
        "Decoding method",
        ConfigFieldType::String,
        true,
        None,
        None,
        None,
        Some(64),
    ),
];
const GIPFORMER_FIELDS: &[ProviderConfigField] = &[
    field(
        "model",
        "Model",
        ConfigFieldType::Select,
        true,
        Some(CapabilitySource::Models),
        None,
        None,
        Some(128),
    ),
    field(
        "language",
        "Language",
        ConfigFieldType::Select,
        true,
        Some(CapabilitySource::Languages),
        None,
        None,
        Some(32),
    ),
    field(
        "num_threads",
        "Threads",
        ConfigFieldType::Integer,
        true,
        None,
        Some(1),
        Some(128),
        None,
    ),
    field(
        "decoding_method",
        "Decoding method",
        ConfigFieldType::String,
        true,
        None,
        None,
        None,
        Some(64),
    ),
    field(
        "max_active_paths",
        "Maximum active paths",
        ConfigFieldType::Integer,
        true,
        None,
        Some(1),
        Some(10_000),
        None,
    ),
];
const OPENAI_FIELDS: &[ProviderConfigField] = &[
    field(
        "base_url",
        "Base URL",
        ConfigFieldType::String,
        true,
        None,
        None,
        None,
        Some(2048),
    ),
    field(
        "model",
        "Model",
        ConfigFieldType::String,
        true,
        None,
        None,
        None,
        Some(256),
    ),
    field(
        "timeout_ms",
        "Timeout (ms)",
        ConfigFieldType::Integer,
        false,
        None,
        Some(1),
        Some(120_000),
        None,
    ),
];
const CHILLAUDIO_FIELDS: &[ProviderConfigField] = &[
    field(
        "ws_url",
        "WebSocket URL",
        ConfigFieldType::String,
        true,
        None,
        None,
        None,
        Some(2048),
    ),
    field(
        "voice",
        "Voice",
        ConfigFieldType::String,
        true,
        None,
        None,
        None,
        Some(128),
    ),
    field(
        "timeout_ms",
        "Timeout (ms)",
        ConfigFieldType::Integer,
        false,
        None,
        Some(1),
        Some(120_000),
        None,
    ),
    field(
        "preload",
        "Preload",
        ConfigFieldType::Boolean,
        false,
        None,
        None,
        None,
        None,
    ),
];
const ZEROTTS_FIELDS: &[ProviderConfigField] = &[
    field(
        "model",
        "Model",
        ConfigFieldType::Select,
        true,
        Some(CapabilitySource::Models),
        None,
        None,
        Some(128),
    ),
    field(
        "voice",
        "Voice",
        ConfigFieldType::Select,
        true,
        Some(CapabilitySource::Voices),
        None,
        None,
        Some(128),
    ),
    field(
        "language",
        "Language",
        ConfigFieldType::Select,
        true,
        Some(CapabilitySource::Languages),
        None,
        None,
        Some(32),
    ),
    field(
        "num_threads",
        "Threads",
        ConfigFieldType::Integer,
        true,
        None,
        Some(1),
        Some(128),
        None,
    ),
    field(
        "preload",
        "Preload",
        ConfigFieldType::Boolean,
        false,
        None,
        None,
        None,
        None,
    ),
    ProviderConfigField {
        key: "delivery_mode",
        label: "Delivery mode",
        field_type: ConfigFieldType::Select,
        required: false,
        nullable: false,
        enum_source: None,
        enum_values: Some(&["stream", "file"]),
        minimum: None,
        maximum: None,
        max_length: None,
        description: None,
    },
];

const fn field(
    key: &'static str,
    label: &'static str,
    field_type: ConfigFieldType,
    required: bool,
    enum_source: Option<CapabilitySource>,
    minimum: Option<i64>,
    maximum: Option<i64>,
    max_length: Option<usize>,
) -> ProviderConfigField {
    ProviderConfigField {
        key,
        label,
        field_type,
        required,
        nullable: false,
        enum_source,
        enum_values: None,
        minimum,
        maximum,
        max_length,
        description: None,
    }
}

const DESCRIPTORS: &[ProviderDescriptor] = &[
    ProviderDescriptor {
        adapter: "silero_onnx",
        provider_type: ProviderType::Vad,
        display_name: "Silero ONNX",
        description: "Local Silero voice activity detector.",
        config_schema: ProviderConfigSchema {
            fields: SILERO_FIELDS,
        },
        capabilities: ProviderCapabilities {
            models: Some(MODELS_SILERO),
            voices: None,
            languages: None,
            streaming: Some(true),
            offline: Some(true),
            tool_calling: None,
            vision: None,
            input_sample_rates: Some(&[16_000]),
            channels: Some(&[1]),
            provider_output_sample_rates: None,
            voice_delivery_sample_rates: None,
        },
        discovery: CapabilityDiscoveryMode {
            models: DiscoverySource::Static,
            voices: DiscoverySource::Unsupported,
            languages: DiscoverySource::Unsupported,
        },
    },
    ProviderDescriptor {
        adapter: "zipformer_sherpa",
        provider_type: ProviderType::Asr,
        display_name: "Zipformer Sherpa",
        description: "Local streaming Vietnamese speech recognition.",
        config_schema: ProviderConfigSchema {
            fields: ZIPFORMER_FIELDS,
        },
        capabilities: ProviderCapabilities {
            models: Some(MODELS_ZIPFORMER),
            voices: None,
            languages: Some(VIETNAMESE),
            streaming: Some(true),
            offline: Some(true),
            tool_calling: None,
            vision: None,
            input_sample_rates: Some(&[16_000]),
            channels: Some(&[1]),
            provider_output_sample_rates: None,
            voice_delivery_sample_rates: None,
        },
        discovery: CapabilityDiscoveryMode {
            models: DiscoverySource::Static,
            voices: DiscoverySource::Unsupported,
            languages: DiscoverySource::Static,
        },
    },
    ProviderDescriptor {
        adapter: "gipformer_sherpa_offline",
        provider_type: ProviderType::Asr,
        display_name: "Gipformer Sherpa Offline",
        description: "Local offline Vietnamese speech recognition.",
        config_schema: ProviderConfigSchema {
            fields: GIPFORMER_FIELDS,
        },
        capabilities: ProviderCapabilities {
            models: Some(MODELS_GIPFORMER),
            voices: None,
            languages: Some(VIETNAMESE),
            streaming: Some(false),
            offline: Some(true),
            tool_calling: None,
            vision: None,
            input_sample_rates: Some(&[16_000]),
            channels: Some(&[1]),
            provider_output_sample_rates: None,
            voice_delivery_sample_rates: None,
        },
        discovery: CapabilityDiscoveryMode {
            models: DiscoverySource::Static,
            voices: DiscoverySource::Unsupported,
            languages: DiscoverySource::Bootstrap,
        },
    },
    ProviderDescriptor {
        adapter: "openai",
        provider_type: ProviderType::Llm,
        display_name: "OpenAI-compatible LLM",
        description: "Remote OpenAI-compatible language model.",
        config_schema: ProviderConfigSchema {
            fields: OPENAI_FIELDS,
        },
        capabilities: ProviderCapabilities {
            models: Some(&[]),
            voices: None,
            languages: None,
            streaming: Some(true),
            offline: Some(false),
            tool_calling: Some(true),
            vision: Some(false),
            input_sample_rates: None,
            channels: None,
            provider_output_sample_rates: None,
            voice_delivery_sample_rates: None,
        },
        discovery: CapabilityDiscoveryMode {
            models: DiscoverySource::Unsupported,
            voices: DiscoverySource::Unsupported,
            languages: DiscoverySource::Unsupported,
        },
    },
    ProviderDescriptor {
        adapter: "zerotts_onnx",
        provider_type: ProviderType::Tts,
        display_name: "ZeroTTS",
        description: "Local streaming Vietnamese text to speech.",
        config_schema: ProviderConfigSchema {
            fields: ZEROTTS_FIELDS,
        },
        capabilities: ProviderCapabilities {
            models: Some(MODELS_ZEROTTS),
            voices: Some(MAICHI),
            languages: Some(VIETNAMESE),
            streaming: Some(true),
            offline: Some(true),
            tool_calling: None,
            vision: None,
            input_sample_rates: None,
            channels: Some(&[1]),
            provider_output_sample_rates: Some(&[48_000]),
            voice_delivery_sample_rates: Some(&[24_000]),
        },
        discovery: CapabilityDiscoveryMode {
            models: DiscoverySource::Static,
            voices: DiscoverySource::BootstrapAndRuntime,
            languages: DiscoverySource::Static,
        },
    },
    ProviderDescriptor {
        adapter: "chillaudio_ws",
        provider_type: ProviderType::Tts,
        display_name: "ChillAudio WebSocket",
        description: "Remote ChillAudio text to speech over WebSocket.",
        config_schema: ProviderConfigSchema {
            fields: CHILLAUDIO_FIELDS,
        },
        capabilities: ProviderCapabilities {
            models: None,
            voices: None,
            languages: None,
            streaming: Some(false),
            offline: Some(false),
            tool_calling: None,
            vision: None,
            input_sample_rates: None,
            channels: None,
            provider_output_sample_rates: None,
            voice_delivery_sample_rates: None,
        },
        discovery: CapabilityDiscoveryMode {
            models: DiscoverySource::Unsupported,
            voices: DiscoverySource::Unsupported,
            languages: DiscoverySource::Unsupported,
        },
    },
];

pub fn compiled_provider_adapter_registry() -> &'static ProviderAdapterRegistry {
    static REGISTRY: ProviderAdapterRegistry = ProviderAdapterRegistry {
        descriptors: DESCRIPTORS,
    };
    &REGISTRY
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModelSelection {
    #[serde(default)]
    model: Option<String>,
}

fn parse_model_selection(selection: &Value, model: &str) -> Result<(), ProviderInspectError> {
    let selection: ModelSelection = serde_json::from_value(selection.clone())
        .map_err(|_| ProviderInspectError::InvalidSelection)?;
    if selection
        .model
        .is_some_and(|selected| selected.len() > 128 || selected != model)
    {
        return Err(ProviderInspectError::InvalidSelection);
    }
    Ok(())
}

fn zerotts_bootstrap(selection: &Value) -> Result<DiscoveredCapabilities, ProviderInspectError> {
    bootstrap_for_model(
        selection,
        "zerotts_default",
        MODELS_ZEROTTS,
        MAICHI,
        VIETNAMESE,
    )
}

fn gipformer_bootstrap(selection: &Value) -> Result<DiscoveredCapabilities, ProviderInspectError> {
    bootstrap_for_model(
        selection,
        "gipformer15_vi_int8",
        MODELS_GIPFORMER,
        &[],
        VIETNAMESE,
    )
}

fn bootstrap_for_model(
    selection: &Value,
    model: &str,
    models: &'static [ModelOption],
    voices: &'static [VoiceOption],
    languages: &'static [LanguageOption],
) -> Result<DiscoveredCapabilities, ProviderInspectError> {
    parse_model_selection(selection, model)?;
    Ok(DiscoveredCapabilities {
        models,
        voices,
        languages,
    })
}
