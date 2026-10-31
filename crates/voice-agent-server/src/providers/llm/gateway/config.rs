use std::time::Duration;

use orchestrator_agent_rs::RequestLimits;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::{
    config::defaults::default_llm_timeout_ms,
    providers::llm::{valid_generation_bounds, valid_llm_tags},
    services::llm_gateway::{GatewayBinding, GatewayVendor},
};

pub const ADAPTER_NAME: &str = "llm_gateway";
pub const TIMEOUT_PROFILES: &[&str] = &["interactive", "standard", "agentic", "custom"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TimeoutProfile {
    Interactive,
    Standard,
    Agentic,
    Custom,
}

impl TimeoutProfile {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "interactive" => Some(Self::Interactive),
            "standard" => Some(Self::Standard),
            "agentic" => Some(Self::Agentic),
            "custom" => Some(Self::Custom),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Interactive => "interactive",
            Self::Standard => "standard",
            Self::Agentic => "agentic",
            Self::Custom => "custom",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CustomLimits {
    pub first_byte_ms: Option<u64>,
    pub idle_ms: Option<u64>,
    pub max_total_ms: Option<u64>,
}

pub fn resolve(
    profile: Option<TimeoutProfile>,
    custom: CustomLimits,
    legacy_timeout_ms: u64,
) -> RequestLimits {
    let (base_first_byte_ms, base_idle_ms, base_max_total_ms) = match profile {
        Some(TimeoutProfile::Interactive) => (30_000, 30_000, 120_000),
        Some(TimeoutProfile::Agentic) => (180_000, 180_000, 1_800_000),
        Some(TimeoutProfile::Standard) | Some(TimeoutProfile::Custom) | None => {
            (120_000, 90_000, 600_000)
        }
    };
    let first_byte_ms = custom.first_byte_ms.unwrap_or(base_first_byte_ms);
    let idle_ms = custom.idle_ms.unwrap_or(base_idle_ms);
    let max_total_ms = custom
        .max_total_ms
        .unwrap_or_else(|| base_max_total_ms.max(legacy_timeout_ms));
    RequestLimits {
        first_byte: Duration::from_millis(first_byte_ms),
        idle: Duration::from_millis(idle_ms),
        max_total: Duration::from_millis(max_total_ms),
    }
}

pub fn valid_timeout_bounds(
    profile: Option<TimeoutProfile>,
    custom: CustomLimits,
    legacy_timeout_ms: u64,
) -> bool {
    let valid_first = custom
        .first_byte_ms
        .is_none_or(|v| (1_000..=3_600_000).contains(&v));
    let valid_idle = custom
        .idle_ms
        .is_none_or(|v| (1_000..=3_600_000).contains(&v));
    let valid_total = custom
        .max_total_ms
        .is_none_or(|v| (1_000..=7_200_000).contains(&v));
    if !valid_first || !valid_idle || !valid_total {
        return false;
    }
    let resolved = resolve(profile, custom, legacy_timeout_ms);
    resolved.idle <= resolved.max_total
}

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
    pub timeout_profile: Option<TimeoutProfile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_byte_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_total_ms: Option<u64>,
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
    pub fn default_timeout_profile(&self) -> TimeoutProfile {
        if matches!(&self.gateway_binding, GatewayBinding::Pool(GatewayVendor::Muse))
            || self.model.trim().to_ascii_lowercase().starts_with("muse")
        {
            TimeoutProfile::Agentic
        } else {
            TimeoutProfile::Standard
        }
    }

    pub fn resolved_limits(&self) -> RequestLimits {
        resolve(
            Some(
                self.timeout_profile
                    .unwrap_or_else(|| self.default_timeout_profile()),
            ),
            CustomLimits {
                first_byte_ms: self.first_byte_ms,
                idle_ms: self.idle_ms,
                max_total_ms: self.max_total_ms,
            },
            self.timeout_ms,
        )
    }

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
            && valid_timeout_bounds(
                Some(
                    self.timeout_profile
                        .unwrap_or_else(|| self.default_timeout_profile()),
                ),
                CustomLimits {
                    first_byte_ms: self.first_byte_ms,
                    idle_ms: self.idle_ms,
                    max_total_ms: self.max_total_ms,
                },
                self.timeout_ms,
            )
            && self
                .account_concurrency
                .is_none_or(|c| (1..=8).contains(&c))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_resolve_legacy_60s_does_not_cap_standard_or_agentic() {
        let raw = r#"{"gateway_binding":"pool:openai_codex","model":"gpt-5.3-codex","timeout_ms":60000}"#;
        let cfg: LlmGatewayConfig = serde_json::from_str(raw).expect("parse legacy config");
        assert!(cfg.valid());
        let limits = cfg.resolved_limits();
        assert_eq!(limits.first_byte, Duration::from_secs(120));
        assert_eq!(limits.idle, Duration::from_secs(90));
        assert_eq!(limits.max_total, Duration::from_secs(600));

        let muse_raw = r#"{"gateway_binding":"pool:muse","model":"muse-chat","timeout_ms":60000}"#;
        let muse_cfg: LlmGatewayConfig = serde_json::from_str(muse_raw).expect("parse muse");
        assert!(muse_cfg.valid());
        let muse_limits = muse_cfg.resolved_limits();
        assert_eq!(muse_limits.first_byte, Duration::from_secs(180));
        assert_eq!(muse_limits.idle, Duration::from_secs(180));
        assert_eq!(muse_limits.max_total, Duration::from_secs(1800));
    }

    #[test]
    fn profile_resolve_legacy_larger_than_default_raises_max_total() {
        let limits = resolve(Some(TimeoutProfile::Standard), CustomLimits::default(), 900_000);
        assert_eq!(limits.first_byte, Duration::from_secs(120));
        assert_eq!(limits.idle, Duration::from_secs(90));
        assert_eq!(limits.max_total, Duration::from_secs(900));
    }

    #[test]
    fn profile_resolve_custom_and_validation() {
        let custom = CustomLimits {
            first_byte_ms: Some(45_000),
            idle_ms: Some(30_000),
            max_total_ms: Some(300_000),
        };
        assert!(valid_timeout_bounds(
            Some(TimeoutProfile::Custom),
            custom,
            60_000
        ));
        let limits = resolve(Some(TimeoutProfile::Custom), custom, 60_000);
        assert_eq!(limits.first_byte, Duration::from_secs(45));
        assert_eq!(limits.idle, Duration::from_secs(30));
        assert_eq!(limits.max_total, Duration::from_secs(300));

        // idle > max_total is rejected
        assert!(!valid_timeout_bounds(
            Some(TimeoutProfile::Custom),
            CustomLimits {
                first_byte_ms: Some(30_000),
                idle_ms: Some(120_000),
                max_total_ms: Some(60_000),
            },
            60_000,
        ));
    }
}
