//! OpenCode Free LLM preset, using the upstream OpenAI-compatible Zen chat API.
//!
//! The free catalog changes over time. We advertise known model IDs for the admin
//! UI, but accept future `-free` chat model IDs in configuration without a release.
use serde::{Deserialize, Serialize};

use crate::providers::{
    LlmError, LlmProvider,
    capabilities::{
        CapabilityDiscoveryMode, CapabilitySource, DiscoverySource, ModelOption,
        ProviderCapabilities,
    },
    descriptor::{
        ConfigFieldType, ProviderConfigField, ProviderConfigSchema, ProviderDescriptor,
        ProviderType, field,
    },
    llm::{ConfiguredOpenAiLlm, LlmEventStream, LlmRequest},
    registry::ProviderAdapterRegistration,
};

pub const BASE_URL: &str = "https://opencode.ai/zen/v1";
pub const PUBLIC_KEY: &str = "public";

fn default_model() -> String {
    "nemotron-3-ultra-free".to_owned()
}

fn default_timeout_ms() -> u64 {
    60_000
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OpenCodeFreeConfig {
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

impl OpenCodeFreeConfig {
    pub fn valid(&self) -> bool {
        self.model.len() <= 256
            && self.model.ends_with("-free")
            && self
                .model
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'.' | b'_'))
            && (1_000..=120_000).contains(&self.timeout_ms)
    }
}

pub(crate) struct OpenCodeFreeProvider {
    inner: ConfiguredOpenAiLlm,
}

impl OpenCodeFreeProvider {
    pub(crate) fn build(config: &OpenCodeFreeConfig) -> Result<Self, LlmError> {
        if !config.valid() {
            return Err(LlmError::Failed);
        }
        Ok(Self {
            // 9Router's no-account OpenCode path uses the non-secret "public" token.
            // There is no persisted credential and the endpoint cannot be overridden.
            inner: ConfiguredOpenAiLlm::build(
                PUBLIC_KEY,
                BASE_URL,
                &config.model,
                config.timeout_ms.div_ceil(1_000),
            )?,
        })
    }
}

#[async_trait::async_trait]
impl LlmProvider for OpenCodeFreeProvider {
    fn adapter(&self) -> &'static str {
        "opencode_free"
    }

    async fn stream(&self, request: LlmRequest) -> Result<LlmEventStream, LlmError> {
        self.inner.stream(request).await
    }
}

// Snapshot of currently advertised free Chat Completions models. Models come
// from https://opencode.ai/zen/v1/models and may become unavailable at any time.
// No assumption of quota availability or guaranteed tool support is made.
const FREE_MODELS: &[ModelOption] = &[
    ModelOption { id: "nemotron-3-ultra-free", name: "Nemotron 3 Ultra Free", description: None },
    ModelOption { id: "mimo-v2.5-free", name: "MiMo V2.5 Free", description: None },
    ModelOption { id: "deepseek-v4-flash-free", name: "DeepSeek V4 Flash Free", description: None },
    ModelOption { id: "ling-3.0-flash-free", name: "Ling 3.0 Flash Free", description: None },
    ModelOption { id: "ling-3.0-tiny-free", name: "Ling 3.0 Tiny Free", description: None },
    ModelOption { id: "north-mini-code-free", name: "North Mini Code Free", description: None },
    ModelOption { id: "laguna-s-2.1-free", name: "Laguna S 2.1 Free", description: None },
    ModelOption { id: "longcat-2.0-free", name: "LongCat 2.0 Free", description: None },
];

const FIELDS: &[ProviderConfigField] = &[
    field(
        "model",
        "Free model",
        ConfigFieldType::Select,
        true,
        Some(CapabilitySource::Models),
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
        Some(120_000),
        None,
    ),
];

pub static DESCRIPTOR: ProviderDescriptor = ProviderDescriptor {
    adapter: "opencode_free",
    provider_type: ProviderType::Llm,
    display_name: "OpenCode Free",
    description: "OpenCode Zen free Chat Completions models; no personal API key required.",
    config_schema: ProviderConfigSchema { fields: FIELDS },
    capabilities: ProviderCapabilities {
        models: Some(FREE_MODELS),
        voices: None,
        languages: None,
        streaming: Some(true),
        offline: Some(false),
        tool_calling: None,
        vision: Some(false),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_free_chat_ids_and_rejects_paid_and_unsafe_ids() {
        assert!(OpenCodeFreeConfig { model: "nemotron-3-ultra-free".into(), timeout_ms: 60_000 }.valid());
        assert!(OpenCodeFreeConfig { model: "future-2027-free".into(), timeout_ms: 60_000 }.valid());
        assert!(!OpenCodeFreeConfig { model: "claude-opus-4-5".into(), timeout_ms: 60_000 }.valid());
        assert!(!OpenCodeFreeConfig { model: "a/../../b-free".into(), timeout_ms: 60_000 }.valid());
    }

    #[tokio::test]
    async fn builds_without_network_or_user_credentials() {
        let config = OpenCodeFreeConfig { model: "nemotron-3-ultra-free".into(), timeout_ms: 30_000 };
        let provider = OpenCodeFreeProvider::build(&config).unwrap();
        assert_eq!(provider.adapter(), "opencode_free");
    }
}
