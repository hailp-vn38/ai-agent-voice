//! OpenCode Free LLM preset: Voice Chat Completions and opt-in Agent Job Responses.
//!
//! Model IDs are entered directly; upstream availability can change at any time.
use llm::providers::openai_compatible::{OpenAICompatibleProvider, OpenAIProviderConfig};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::providers::{
    LlmError, LlmProvider,
    capabilities::{CapabilityDiscoveryMode, DiscoverySource, ProviderCapabilities},
    descriptor::{
        ConfigFieldType, ProviderConfigField, ProviderConfigSchema, ProviderDescriptor,
        ProviderType, field,
    },
    llm::{LlmEvent, LlmEventStream, LlmRequest, ToolDefinition, stream_openai_request},
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

pub(crate) fn valid_free_model_id(model: &str) -> bool {
    model.len() <= 256
        && model.ends_with("-free")
        && model.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'.' | b'_')
        })
}

impl OpenCodeFreeConfig {
    pub fn valid(&self) -> bool {
        valid_free_model_id(&self.model) && (1_000..=120_000).contains(&self.timeout_ms)
    }
}

pub(crate) struct OpenCodeFreeProvider {
    model: String,
    timeout_seconds: u64,
}

impl OpenCodeFreeProvider {
    pub(crate) fn build(config: &OpenCodeFreeConfig) -> Result<Self, LlmError> {
        if !config.valid() {
            return Err(LlmError::Failed);
        }
        Ok(Self {
            model: config.model.clone(),
            timeout_seconds: config.timeout_ms.div_ceil(1_000),
        })
    }

    fn chat_client(&self, request: &LlmRequest) -> OpenAICompatibleProvider<ZenChat> {
        let system = request.messages.iter().find_map(|message| match message {
            super::ChatMessage::System { content } => Some(content.clone()),
            _ => None,
        });
        OpenAICompatibleProvider::<ZenChat>::new(
            PUBLIC_KEY,
            Some(BASE_URL.into()),
            Some(self.model.clone()),
            None,
            None,
            Some(self.timeout_seconds),
            system,
            None,
            None,
            None,
            request
                .tools
                .is_empty()
                .then_some(llm::chat::ToolChoice::None),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
    }
}

#[async_trait::async_trait]
impl LlmProvider for OpenCodeFreeProvider {
    fn supports_tools(&self) -> bool {
        true
    }
    fn adapter(&self) -> &'static str {
        "opencode_free"
    }

    async fn stream(&self, mut request: LlmRequest) -> Result<LlmEventStream, LlmError> {
        use futures_util::StreamExt;
        let client = self.chat_client(&request);
        let placeholders = add_fingerprint_tools(&mut request);
        let stream = stream_openai_request(&client, request, self.adapter()).await?;
        Ok(Box::pin(stream.map(move |event| {
            reject_placeholder_call(event, &placeholders)
        })))
    }
}

struct ZenChat;

impl OpenAIProviderConfig for ZenChat {
    const PROVIDER_NAME: &'static str = "OpenCode Free";
    const DEFAULT_BASE_URL: &'static str = BASE_URL;
    const DEFAULT_MODEL: &'static str = "nemotron-3-ultra-free";
    const SUPPORTS_STREAM_OPTIONS: bool = true;

    fn custom_headers() -> Option<Vec<(String, String)>> {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let descending = (!(millis << 12)) & 0xffff_ffff_ffff;
        // shortcut: fresh session per request; preserve affinity when LlmRequest carries conversation identity.
        let session = format!(
            "ses_{descending:012x}{}",
            &uuid::Uuid::new_v4().simple().to_string()[..14]
        );
        Some(vec![
            ("User-Agent".into(), "opencode/1.18.31".into()),
            ("x-opencode-session".into(), session),
            ("x-opencode-client".into(), "desktop".into()),
            ("x-opencode-project".into(), "global".into()),
        ])
    }
}

const FINGERPRINT_TOOLS: [&str; 4] = ["bash", "glob", "grep", "read"];

fn add_fingerprint_tools(request: &mut LlmRequest) -> Vec<&'static str> {
    let mut placeholders = Vec::new();
    for name in FINGERPRINT_TOOLS {
        if !request.tools.iter().any(|tool| tool.name == name) {
            request.tools.push(ToolDefinition {
                name: name.into(),
                description: "Unavailable. Do not call this tool.".into(),
                parameters: serde_json::json!({"type": "object", "properties": {}}),
            });
            placeholders.push(name);
        }
    }
    placeholders
}

fn reject_placeholder_call(
    event: Result<LlmEvent, LlmError>,
    placeholders: &[&str],
) -> Result<LlmEvent, LlmError> {
    if let Ok(LlmEvent::ToolCall(call)) = &event
        && placeholders.contains(&call.name.as_str())
    {
        return Err(LlmError::Failed);
    }
    event
}

const FIELDS: &[ProviderConfigField] = &[
    field(
        "model",
        "Free model",
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
        Some(1_000),
        Some(120_000),
        None,
    ),
];

pub static DESCRIPTOR: ProviderDescriptor = ProviderDescriptor {
    adapter: "opencode_free",
    provider_type: ProviderType::Llm,
    display_name: "OpenCode Free",
    description: "OpenCode Zen free models via Chat Completions.",
    config_schema: ProviderConfigSchema { fields: FIELDS },
    capabilities: ProviderCapabilities {
        models: None,
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
        models: DiscoverySource::Unsupported,
        voices: DiscoverySource::Unsupported,
        languages: DiscoverySource::Unsupported,
    },
};

pub static REGISTRATION: ProviderAdapterRegistration =
    ProviderAdapterRegistration::remote(&DESCRIPTOR, None);

#[cfg(test)]
mod tests;
