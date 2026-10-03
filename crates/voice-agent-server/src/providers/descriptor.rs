//! Provider descriptor wire models. Adapter-owned metadata lives beside each adapter.

use serde::{Deserialize, Serialize};

use super::capabilities::{CapabilityDiscoveryMode, CapabilitySource, ProviderCapabilities};

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderType {
    Vad,
    Asr,
    Llm,
    Tts,
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

#[allow(clippy::too_many_arguments)] // Const adapter descriptors are clearer as a flat declaration.
pub(crate) const fn field(
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
