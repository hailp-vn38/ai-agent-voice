use crate::providers::{
    capabilities::{CapabilityDiscoveryMode, DiscoverySource, ModelOption, ProviderCapabilities},
    descriptor::{
        field, multi_select_field, select_field, ConfigFieldType, ProviderConfigField,
        ProviderConfigSchema, ProviderDescriptor, ProviderType,
    },
    llm::{LLM_CAPABILITY_TAGS, REASONING_EFFORTS},
    registry::ProviderAdapterRegistration,
};

use super::config::{ADAPTER_NAME, MUSE_MODELS};

const MODEL_OPTIONS: &[ModelOption] = &[
    ModelOption {
        id: MUSE_MODELS[0],
        name: "Muse Chat & Vision",
        description: Some("Conversational chat and image understanding on muse.ai"),
    },
    ModelOption {
        id: MUSE_MODELS[1],
        name: "Muse Image Generation & Edit",
        description: Some("Text-to-Image and multi-reference Image-to-Image generation"),
    },
    ModelOption {
        id: MUSE_MODELS[2],
        name: "Muse Video Generation",
        description: Some("Text-to-Video and First-Frame Image-to-Video generation"),
    },
];

const FIELDS: &[ProviderConfigField] = &[
    field(
        "model",
        "Model",
        ConfigFieldType::String,
        false,
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
        Some(1_000),
        Some(600_000),
        None,
    ),
    field(
        "max_tokens",
        "Max output tokens",
        ConfigFieldType::Integer,
        false,
        None,
        Some(256),
        Some(65_536),
        None,
    ),
    field(
        "context_limit_tokens",
        "Context window (tokens)",
        ConfigFieldType::Integer,
        false,
        None,
        Some(4_096),
        Some(2_000_000),
        None,
    ),
    select_field("reasoning_effort", "Reasoning effort", REASONING_EFFORTS),
    field(
        "temperature_percent",
        "Temperature (0-200%)",
        ConfigFieldType::Integer,
        false,
        None,
        Some(0),
        Some(200),
        None,
    ),
    multi_select_field("tags", "Capability tags", LLM_CAPABILITY_TAGS),
    field(
        "account_concurrency",
        "Max tabs per account",
        ConfigFieldType::Integer,
        false,
        None,
        Some(1),
        Some(8),
        None,
    ),
    field(
        "cdp_port",
        "Chromium CDP port",
        ConfigFieldType::Integer,
        false,
        None,
        Some(1024),
        Some(65535),
        None,
    ),
    field(
        "headless",
        "Headless browser",
        ConfigFieldType::Boolean,
        false,
        None,
        None,
        None,
        None,
    ),
];

pub static DESCRIPTOR: ProviderDescriptor = ProviderDescriptor {
    adapter: ADAPTER_NAME,
    provider_type: ProviderType::Llm,
    display_name: "Muse AI (Browser CDP)",
    description: "Multimodal chat, vision, image generation/editing, and video generation via muse.ai browser automation.",
    config_schema: ProviderConfigSchema { fields: FIELDS },
    capabilities: ProviderCapabilities {
        models: Some(MODEL_OPTIONS),
        voices: None,
        languages: None,
        streaming: Some(true),
        offline: Some(false),
        tool_calling: Some(false),
        vision: Some(true),
        input_sample_rates: None,
        channels: None,
        provider_output_sample_rates: None,
        voice_delivery_sample_rates: None,
    },
    discovery: CapabilityDiscoveryMode {
        models: DiscoverySource::Static,
        voices: DiscoverySource::Unsupported,
        languages: DiscoverySource::Unsupported,
    },
};

pub static REGISTRATION: ProviderAdapterRegistration =
    ProviderAdapterRegistration::remote(&DESCRIPTOR, None);
