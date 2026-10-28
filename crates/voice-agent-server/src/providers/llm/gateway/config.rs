use serde::{Deserialize, Serialize};
use url::Url;

use crate::{
    config::defaults::default_llm_timeout_ms,
    providers::llm::{valid_generation_bounds, valid_llm_tags},
    services::llm_gateway::GatewayBinding,
};

pub const ADAPTER_NAME: &str = "llm_gateway";

fn default_gateway_model() -> String {
    "claude-sonnet-4-20250514".to_owned()
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LlmGatewayConfig {
    pub gateway_binding: GatewayBinding,
    #[serde(default = "default_gateway_model")]
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<Url>,
    #[serde(default = "default_llm_timeout_ms")]
    pub timeout_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_limit_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature_percent: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
}

impl LlmGatewayConfig {
    pub fn valid(&self) -> bool {
        let model_trimmed = self.model.trim();
        let valid_url = self
            .base_url
            .as_ref()
            .is_none_or(|u| matches!(u.scheme(), "http" | "https"));
        valid_url
            && !model_trimmed.is_empty()
            && model_trimmed.len() <= 256
            && (1_000..=600_000).contains(&self.timeout_ms)
            && valid_generation_bounds(
                self.max_tokens,
                self.context_limit_tokens,
                self.reasoning_effort.as_deref(),
                self.temperature_percent,
            )
            && valid_llm_tags(self.tags.as_deref())
    }
}
