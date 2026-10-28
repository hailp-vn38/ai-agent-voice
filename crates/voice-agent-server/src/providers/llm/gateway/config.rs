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
    /// Maximum concurrent browser tabs per Muse account (1..=8, default: 2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_concurrency: Option<usize>,
    /// Local Chromium CDP port for Muse (1024..=65535, default: 19222).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cdp_port: Option<u16>,
    /// Run Chromium in headless mode for Muse (`--headless=new`, default: true).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headless: Option<bool>,
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
            && self.account_concurrency.is_none_or(|c| (1..=8).contains(&c))
            && self.cdp_port.is_none_or(|p| p >= 1024)
            && valid_generation_bounds(
                self.max_tokens,
                self.context_limit_tokens,
                self.reasoning_effort.as_deref(),
                self.temperature_percent,
            )
            && valid_llm_tags(self.tags.as_deref())
    }

    pub fn effective_account_concurrency(&self) -> usize {
        self.account_concurrency.unwrap_or(2).clamp(1, 8)
    }

    pub fn effective_cdp_port(&self) -> u16 {
        self.cdp_port.unwrap_or(19222).max(1024)
    }

    pub fn effective_headless(&self) -> bool {
        self.headless.unwrap_or(true)
    }
}
