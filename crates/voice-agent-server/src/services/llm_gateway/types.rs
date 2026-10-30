use std::{fmt, str::FromStr};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::config::SecretString;
use crate::database::{
    credentials,
    secrets::{SecretRef, SecretResolver, SecretValue},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayVendor {
    Anthropic,
    OpenaiCodex,
    Gemini,
    GithubCopilot,
    Muse,
}

impl GatewayVendor {
    pub const ALL: [Self; 5] = [
        Self::Anthropic,
        Self::OpenaiCodex,
        Self::Gemini,
        Self::GithubCopilot,
        Self::Muse,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::OpenaiCodex => "openai_codex",
            Self::Gemini => "gemini",
            Self::GithubCopilot => "github_copilot",
            Self::Muse => "muse",
        }
    }

    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Anthropic => "Anthropic Claude",
            Self::OpenaiCodex => "OpenAI Codex (ChatGPT)",
            Self::Gemini => "Google Gemini",
            Self::GithubCopilot => "GitHub Copilot",
            Self::Muse => "Muse AI (Browser CDP)",
        }
    }

    pub const fn default_base_url(self) -> &'static str {
        match self {
            Self::Anthropic => "https://api.anthropic.com/v1",
            Self::OpenaiCodex => "https://chatgpt.com/backend-api/codex",
            Self::Gemini => "https://daily-cloudcode-pa.googleapis.com/v1internal",
            Self::GithubCopilot => "https://api.githubcopilot.com",
            Self::Muse => "https://muse.ai",
        }
    }

    pub const fn login_mode(self) -> OAuthLoginMode {
        match self {
            Self::Anthropic | Self::OpenaiCodex | Self::Gemini | Self::Muse => OAuthLoginMode::Pkce,
            Self::GithubCopilot => OAuthLoginMode::DeviceCode,
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "anthropic" => Some(Self::Anthropic),
            "openai_codex" => Some(Self::OpenaiCodex),
            "gemini" => Some(Self::Gemini),
            "github_copilot" => Some(Self::GithubCopilot),
            "muse" => Some(Self::Muse),
            _ => None,
        }
    }

    pub const fn preset_models(self) -> &'static [GatewayVendorModelPreset] {
        match self {
            Self::Anthropic => &[
                GatewayVendorModelPreset {
                    model: "claude-sonnet-4-6",
                    display_name: "Claude Sonnet 4.6",
                    tags: &["chat", "reasoning", "vision"],
                    supports_reasoning_effort: false,
                    default_max_tokens: Some(8192),
                    default_context_limit_tokens: Some(200_000),
                    default_reasoning_effort: None,
                    default_temperature_percent: Some(70),
                    default_timeout_ms: Some(90_000),
                },
                GatewayVendorModelPreset {
                    model: "claude-opus-4-6",
                    display_name: "Claude Opus 4.6",
                    tags: &["chat", "reasoning", "vision"],
                    supports_reasoning_effort: false,
                    default_max_tokens: Some(8192),
                    default_context_limit_tokens: Some(200_000),
                    default_reasoning_effort: None,
                    default_temperature_percent: Some(70),
                    default_timeout_ms: Some(120_000),
                },
                GatewayVendorModelPreset {
                    model: "claude-sonnet-4-5-20250929",
                    display_name: "Claude Sonnet 4.5",
                    tags: &["chat", "reasoning", "vision"],
                    supports_reasoning_effort: false,
                    default_max_tokens: Some(8192),
                    default_context_limit_tokens: Some(200_000),
                    default_reasoning_effort: None,
                    default_temperature_percent: Some(70),
                    default_timeout_ms: Some(90_000),
                },
                GatewayVendorModelPreset {
                    model: "claude-opus-4-5-20251101",
                    display_name: "Claude Opus 4.5",
                    tags: &["chat", "reasoning", "vision"],
                    supports_reasoning_effort: false,
                    default_max_tokens: Some(8192),
                    default_context_limit_tokens: Some(200_000),
                    default_reasoning_effort: None,
                    default_temperature_percent: Some(70),
                    default_timeout_ms: Some(120_000),
                },
                GatewayVendorModelPreset {
                    model: "claude-haiku-4-5-20251001",
                    display_name: "Claude Haiku 4.5",
                    tags: &["chat", "vision"],
                    supports_reasoning_effort: false,
                    default_max_tokens: Some(4096),
                    default_context_limit_tokens: Some(200_000),
                    default_reasoning_effort: None,
                    default_temperature_percent: Some(70),
                    default_timeout_ms: Some(60_000),
                },
                GatewayVendorModelPreset {
                    model: "claude-opus-4-1-20250805",
                    display_name: "Claude Opus 4.1",
                    tags: &["chat", "reasoning", "vision"],
                    supports_reasoning_effort: false,
                    default_max_tokens: Some(8192),
                    default_context_limit_tokens: Some(200_000),
                    default_reasoning_effort: None,
                    default_temperature_percent: Some(70),
                    default_timeout_ms: Some(120_000),
                },
                GatewayVendorModelPreset {
                    model: "claude-sonnet-4-20250514",
                    display_name: "Claude Sonnet 4",
                    tags: &["chat", "reasoning", "vision"],
                    supports_reasoning_effort: false,
                    default_max_tokens: Some(4096),
                    default_context_limit_tokens: Some(200_000),
                    default_reasoning_effort: None,
                    default_temperature_percent: Some(70),
                    default_timeout_ms: Some(60_000),
                },
                GatewayVendorModelPreset {
                    model: "claude-opus-4-20250514",
                    display_name: "Claude Opus 4",
                    tags: &["chat", "reasoning", "vision"],
                    supports_reasoning_effort: false,
                    default_max_tokens: Some(4096),
                    default_context_limit_tokens: Some(200_000),
                    default_reasoning_effort: None,
                    default_temperature_percent: Some(70),
                    default_timeout_ms: Some(90_000),
                },
                GatewayVendorModelPreset {
                    model: "claude-3-7-sonnet-20250219",
                    display_name: "Claude 3.7 Sonnet",
                    tags: &["chat", "reasoning", "vision"],
                    supports_reasoning_effort: false,
                    default_max_tokens: Some(4096),
                    default_context_limit_tokens: Some(200_000),
                    default_reasoning_effort: None,
                    default_temperature_percent: Some(70),
                    default_timeout_ms: Some(60_000),
                },
                GatewayVendorModelPreset {
                    model: "claude-3-5-haiku-20241022",
                    display_name: "Claude 3.5 Haiku",
                    tags: &["chat", "vision"],
                    supports_reasoning_effort: false,
                    default_max_tokens: Some(4096),
                    default_context_limit_tokens: Some(200_000),
                    default_reasoning_effort: None,
                    default_temperature_percent: Some(70),
                    default_timeout_ms: Some(45_000),
                },
            ],
            Self::OpenaiCodex => &[
                GatewayVendorModelPreset {
                    model: "gpt-6-astra",
                    display_name: "GPT-6 Astra",
                    tags: &["chat", "reasoning", "vision"],
                    supports_reasoning_effort: true,
                    default_max_tokens: Some(128_000),
                    default_context_limit_tokens: Some(1_050_000),
                    default_reasoning_effort: Some("high"),
                    default_temperature_percent: None,
                    default_timeout_ms: Some(120_000),
                },
                GatewayVendorModelPreset {
                    model: "gpt-6.1-sol",
                    display_name: "GPT-6.1 Sol",
                    tags: &["chat", "reasoning", "vision"],
                    supports_reasoning_effort: true,
                    default_max_tokens: Some(128_000),
                    default_context_limit_tokens: Some(1_050_000),
                    default_reasoning_effort: Some("high"),
                    default_temperature_percent: None,
                    default_timeout_ms: Some(90_000),
                },
                GatewayVendorModelPreset {
                    model: "gpt-6.1-terra",
                    display_name: "GPT-6.1 Terra",
                    tags: &["chat", "reasoning", "vision"],
                    supports_reasoning_effort: true,
                    default_max_tokens: Some(128_000),
                    default_context_limit_tokens: Some(1_050_000),
                    default_reasoning_effort: Some("medium"),
                    default_temperature_percent: None,
                    default_timeout_ms: Some(90_000),
                },
                GatewayVendorModelPreset {
                    model: "gpt-6-sol",
                    display_name: "GPT-6 Sol",
                    tags: &["chat", "reasoning", "vision"],
                    supports_reasoning_effort: true,
                    default_max_tokens: Some(128_000),
                    default_context_limit_tokens: Some(1_050_000),
                    default_reasoning_effort: Some("high"),
                    default_temperature_percent: None,
                    default_timeout_ms: Some(90_000),
                },
                GatewayVendorModelPreset {
                    model: "gpt-6-luna",
                    display_name: "GPT-6 Luna",
                    tags: &["chat", "reasoning", "vision"],
                    supports_reasoning_effort: true,
                    default_max_tokens: Some(128_000),
                    default_context_limit_tokens: Some(1_050_000),
                    default_reasoning_effort: Some("low"),
                    default_temperature_percent: None,
                    default_timeout_ms: Some(60_000),
                },
                GatewayVendorModelPreset {
                    model: "gpt-5.6-sol",
                    display_name: "GPT-5.6 Sol",
                    tags: &["chat", "reasoning", "vision"],
                    supports_reasoning_effort: true,
                    default_max_tokens: Some(128_000),
                    default_context_limit_tokens: Some(1_050_000),
                    default_reasoning_effort: Some("high"),
                    default_temperature_percent: None,
                    default_timeout_ms: Some(90_000),
                },
                GatewayVendorModelPreset {
                    model: "gpt-5.6-terra",
                    display_name: "GPT-5.6 Terra (Codex)",
                    tags: &["chat", "reasoning", "vision"],
                    supports_reasoning_effort: true,
                    default_max_tokens: Some(128_000),
                    default_context_limit_tokens: Some(1_050_000),
                    default_reasoning_effort: Some("high"),
                    default_temperature_percent: None,
                    default_timeout_ms: Some(90_000),
                },
                GatewayVendorModelPreset {
                    model: "gpt-5.6-luna",
                    display_name: "GPT-5.6 Luna",
                    tags: &["chat", "reasoning", "vision"],
                    supports_reasoning_effort: true,
                    default_max_tokens: Some(128_000),
                    default_context_limit_tokens: Some(1_050_000),
                    default_reasoning_effort: Some("low"),
                    default_temperature_percent: None,
                    default_timeout_ms: Some(60_000),
                },
                GatewayVendorModelPreset {
                    model: "gpt-5.5",
                    display_name: "GPT-5.5",
                    tags: &["chat", "reasoning", "vision"],
                    supports_reasoning_effort: true,
                    default_max_tokens: Some(128_000),
                    default_context_limit_tokens: Some(1_050_000),
                    default_reasoning_effort: Some("high"),
                    default_temperature_percent: None,
                    default_timeout_ms: Some(90_000),
                },
                GatewayVendorModelPreset {
                    model: "gpt-5.4",
                    display_name: "GPT-5.4",
                    tags: &["chat", "reasoning", "vision"],
                    supports_reasoning_effort: true,
                    default_max_tokens: Some(128_000),
                    default_context_limit_tokens: Some(1_050_000),
                    default_reasoning_effort: Some("medium"),
                    default_temperature_percent: None,
                    default_timeout_ms: Some(90_000),
                },
                GatewayVendorModelPreset {
                    model: "gpt-5.4-mini",
                    display_name: "GPT-5.4 Mini",
                    tags: &["chat", "reasoning", "vision"],
                    supports_reasoning_effort: true,
                    default_max_tokens: Some(128_000),
                    default_context_limit_tokens: Some(400_000),
                    default_reasoning_effort: Some("medium"),
                    default_temperature_percent: None,
                    default_timeout_ms: Some(60_000),
                },
                GatewayVendorModelPreset {
                    model: "gpt-5.3-codex",
                    display_name: "GPT-5.3 Codex",
                    tags: &["chat", "reasoning", "vision"],
                    supports_reasoning_effort: true,
                    default_max_tokens: Some(128_000),
                    default_context_limit_tokens: Some(400_000),
                    default_reasoning_effort: Some("high"),
                    default_temperature_percent: None,
                    default_timeout_ms: Some(90_000),
                },
                GatewayVendorModelPreset {
                    model: "gpt-5.3-codex-spark",
                    display_name: "GPT-5.3 Codex Spark",
                    tags: &["chat", "reasoning"],
                    supports_reasoning_effort: true,
                    default_max_tokens: Some(64_000),
                    default_context_limit_tokens: Some(128_000),
                    default_reasoning_effort: Some("low"),
                    default_temperature_percent: None,
                    default_timeout_ms: Some(45_000),
                },
                GatewayVendorModelPreset {
                    model: "gpt-5.2-codex",
                    display_name: "GPT-5.2 Codex",
                    tags: &["chat", "reasoning", "vision"],
                    supports_reasoning_effort: true,
                    default_max_tokens: Some(128_000),
                    default_context_limit_tokens: Some(400_000),
                    default_reasoning_effort: Some("medium"),
                    default_temperature_percent: None,
                    default_timeout_ms: Some(90_000),
                },
                GatewayVendorModelPreset {
                    model: "gpt-5.1-codex-max",
                    display_name: "GPT-5.1 Codex Max",
                    tags: &["chat", "reasoning", "vision"],
                    supports_reasoning_effort: true,
                    default_max_tokens: Some(128_000),
                    default_context_limit_tokens: Some(400_000),
                    default_reasoning_effort: Some("high"),
                    default_temperature_percent: None,
                    default_timeout_ms: Some(120_000),
                },
                GatewayVendorModelPreset {
                    model: "gpt-5.1-codex-mini",
                    display_name: "GPT-5.1 Codex Mini",
                    tags: &["chat", "reasoning"],
                    supports_reasoning_effort: true,
                    default_max_tokens: Some(64_000),
                    default_context_limit_tokens: Some(200_000),
                    default_reasoning_effort: Some("low"),
                    default_temperature_percent: None,
                    default_timeout_ms: Some(60_000),
                },
                GatewayVendorModelPreset {
                    model: "gpt-5",
                    display_name: "GPT-5",
                    tags: &["chat", "reasoning", "vision"],
                    supports_reasoning_effort: true,
                    default_max_tokens: Some(128_000),
                    default_context_limit_tokens: Some(400_000),
                    default_reasoning_effort: Some("medium"),
                    default_temperature_percent: None,
                    default_timeout_ms: Some(90_000),
                },
                GatewayVendorModelPreset {
                    model: "gpt-5-mini",
                    display_name: "GPT-5 Mini",
                    tags: &["chat", "reasoning", "vision"],
                    supports_reasoning_effort: true,
                    default_max_tokens: Some(128_000),
                    default_context_limit_tokens: Some(400_000),
                    default_reasoning_effort: Some("low"),
                    default_temperature_percent: None,
                    default_timeout_ms: Some(60_000),
                },
                GatewayVendorModelPreset {
                    model: "codex-mini-latest",
                    display_name: "Codex Mini Latest",
                    tags: &["chat", "reasoning"],
                    supports_reasoning_effort: true,
                    default_max_tokens: Some(100_000),
                    default_context_limit_tokens: Some(200_000),
                    default_reasoning_effort: Some("medium"),
                    default_temperature_percent: None,
                    default_timeout_ms: Some(60_000),
                },
                GatewayVendorModelPreset {
                    model: "o3",
                    display_name: "OpenAI o3",
                    tags: &["chat", "reasoning", "vision"],
                    supports_reasoning_effort: true,
                    default_max_tokens: Some(100_000),
                    default_context_limit_tokens: Some(200_000),
                    default_reasoning_effort: Some("medium"),
                    default_temperature_percent: None,
                    default_timeout_ms: Some(90_000),
                },
                GatewayVendorModelPreset {
                    model: "o3-pro",
                    display_name: "OpenAI o3-pro",
                    tags: &["chat", "reasoning", "vision"],
                    supports_reasoning_effort: true,
                    default_max_tokens: Some(100_000),
                    default_context_limit_tokens: Some(200_000),
                    default_reasoning_effort: Some("high"),
                    default_temperature_percent: None,
                    default_timeout_ms: Some(180_000),
                },
                GatewayVendorModelPreset {
                    model: "o4-mini",
                    display_name: "OpenAI o4-mini",
                    tags: &["chat", "reasoning", "vision"],
                    supports_reasoning_effort: true,
                    default_max_tokens: Some(100_000),
                    default_context_limit_tokens: Some(200_000),
                    default_reasoning_effort: Some("medium"),
                    default_temperature_percent: None,
                    default_timeout_ms: Some(60_000),
                },
                GatewayVendorModelPreset {
                    model: "gpt-4.1",
                    display_name: "GPT-4.1",
                    tags: &["chat", "vision"],
                    supports_reasoning_effort: false,
                    default_max_tokens: Some(32_768),
                    default_context_limit_tokens: Some(1_047_576),
                    default_reasoning_effort: None,
                    default_temperature_percent: Some(70),
                    default_timeout_ms: Some(60_000),
                },
                GatewayVendorModelPreset {
                    model: "gpt-4.1-mini",
                    display_name: "GPT-4.1 Mini",
                    tags: &["chat", "vision"],
                    supports_reasoning_effort: false,
                    default_max_tokens: Some(32_768),
                    default_context_limit_tokens: Some(1_047_576),
                    default_reasoning_effort: None,
                    default_temperature_percent: Some(70),
                    default_timeout_ms: Some(45_000),
                },
            ],
            Self::Gemini => &[
                GatewayVendorModelPreset {
                    model: "gemini-3.8-flash",
                    display_name: "Gemini 3.8 Flash (Antigravity)",
                    tags: &["chat", "reasoning", "vision"],
                    supports_reasoning_effort: true,
                    default_max_tokens: Some(65_000),
                    default_context_limit_tokens: Some(1_000_000),
                    default_reasoning_effort: Some("medium"),
                    default_temperature_percent: Some(70),
                    default_timeout_ms: Some(60_000),
                },
                GatewayVendorModelPreset {
                    model: "gemini-3.8-flash-high",
                    display_name: "Gemini 3.8 Flash High (Antigravity)",
                    tags: &["chat", "reasoning", "vision"],
                    supports_reasoning_effort: true,
                    default_max_tokens: Some(65_000),
                    default_context_limit_tokens: Some(1_000_000),
                    default_reasoning_effort: Some("high"),
                    default_temperature_percent: Some(70),
                    default_timeout_ms: Some(60_000),
                },
                GatewayVendorModelPreset {
                    model: "gemini-4-argon-high",
                    display_name: "Gemini 4 Argon High (Antigravity)",
                    tags: &["chat", "reasoning", "vision"],
                    supports_reasoning_effort: true,
                    default_max_tokens: Some(65_000),
                    default_context_limit_tokens: Some(1_000_000),
                    default_reasoning_effort: Some("high"),
                    default_temperature_percent: Some(70),
                    default_timeout_ms: Some(90_000),
                },
                GatewayVendorModelPreset {
                    model: "gemini-3.1-pro-high",
                    display_name: "Gemini 3.1 Pro High (Antigravity)",
                    tags: &["chat", "reasoning", "vision"],
                    supports_reasoning_effort: true,
                    default_max_tokens: Some(65_000),
                    default_context_limit_tokens: Some(1_000_000),
                    default_reasoning_effort: Some("high"),
                    default_temperature_percent: Some(70),
                    default_timeout_ms: Some(90_000),
                },
                GatewayVendorModelPreset {
                    model: "gemini-3.1-pro-preview",
                    display_name: "Gemini 3.1 Pro Preview",
                    tags: &["chat", "reasoning", "vision"],
                    supports_reasoning_effort: true,
                    default_max_tokens: Some(8192),
                    default_context_limit_tokens: Some(1_000_000),
                    default_reasoning_effort: Some("high"),
                    default_temperature_percent: Some(70),
                    default_timeout_ms: Some(90_000),
                },
                GatewayVendorModelPreset {
                    model: "gemini-3-pro-preview",
                    display_name: "Gemini 3 Pro Preview",
                    tags: &["chat", "reasoning", "vision"],
                    supports_reasoning_effort: true,
                    default_max_tokens: Some(8192),
                    default_context_limit_tokens: Some(1_000_000),
                    default_reasoning_effort: Some("high"),
                    default_temperature_percent: Some(70),
                    default_timeout_ms: Some(90_000),
                },
                GatewayVendorModelPreset {
                    model: "gemini-3-flash-preview",
                    display_name: "Gemini 3 Flash Preview",
                    tags: &["chat", "reasoning", "vision"],
                    supports_reasoning_effort: true,
                    default_max_tokens: Some(8192),
                    default_context_limit_tokens: Some(1_000_000),
                    default_reasoning_effort: Some("medium"),
                    default_temperature_percent: Some(70),
                    default_timeout_ms: Some(60_000),
                },
                GatewayVendorModelPreset {
                    model: "gemini-2.5-pro",
                    display_name: "Gemini 2.5 Pro",
                    tags: &["chat", "reasoning", "vision"],
                    supports_reasoning_effort: true,
                    default_max_tokens: Some(8192),
                    default_context_limit_tokens: Some(1_000_000),
                    default_reasoning_effort: Some("medium"),
                    default_temperature_percent: Some(70),
                    default_timeout_ms: Some(90_000),
                },
                GatewayVendorModelPreset {
                    model: "gemini-2.5-flash",
                    display_name: "Gemini 2.5 Flash",
                    tags: &["chat", "reasoning", "vision"],
                    supports_reasoning_effort: true,
                    default_max_tokens: Some(8192),
                    default_context_limit_tokens: Some(1_000_000),
                    default_reasoning_effort: Some("low"),
                    default_temperature_percent: Some(70),
                    default_timeout_ms: Some(60_000),
                },
                GatewayVendorModelPreset {
                    model: "gemini-2.5-flash-lite",
                    display_name: "Gemini 2.5 Flash-Lite",
                    tags: &["chat", "vision"],
                    supports_reasoning_effort: false,
                    default_max_tokens: Some(4096),
                    default_context_limit_tokens: Some(1_000_000),
                    default_reasoning_effort: None,
                    default_temperature_percent: Some(70),
                    default_timeout_ms: Some(45_000),
                },
                GatewayVendorModelPreset {
                    model: "gemini-2.0-flash",
                    display_name: "Gemini 2.0 Flash",
                    tags: &["chat", "vision"],
                    supports_reasoning_effort: false,
                    default_max_tokens: Some(4096),
                    default_context_limit_tokens: Some(1_000_000),
                    default_reasoning_effort: None,
                    default_temperature_percent: Some(70),
                    default_timeout_ms: Some(45_000),
                },
            ],
            Self::GithubCopilot => &[
                GatewayVendorModelPreset {
                    model: "claude-sonnet-4.5",
                    display_name: "Copilot Claude Sonnet 4.5",
                    tags: &["chat", "reasoning", "vision"],
                    supports_reasoning_effort: false,
                    default_max_tokens: Some(8192),
                    default_context_limit_tokens: Some(200_000),
                    default_reasoning_effort: None,
                    default_temperature_percent: Some(70),
                    default_timeout_ms: Some(90_000),
                },
                GatewayVendorModelPreset {
                    model: "claude-sonnet-4",
                    display_name: "Copilot Claude Sonnet 4",
                    tags: &["chat", "reasoning", "vision"],
                    supports_reasoning_effort: false,
                    default_max_tokens: Some(8192),
                    default_context_limit_tokens: Some(200_000),
                    default_reasoning_effort: None,
                    default_temperature_percent: Some(70),
                    default_timeout_ms: Some(90_000),
                },
                GatewayVendorModelPreset {
                    model: "claude-opus-4",
                    display_name: "Copilot Claude Opus 4",
                    tags: &["chat", "reasoning", "vision"],
                    supports_reasoning_effort: false,
                    default_max_tokens: Some(8192),
                    default_context_limit_tokens: Some(200_000),
                    default_reasoning_effort: None,
                    default_temperature_percent: Some(70),
                    default_timeout_ms: Some(120_000),
                },
                GatewayVendorModelPreset {
                    model: "claude-3.7-sonnet",
                    display_name: "Copilot Claude 3.7 Sonnet",
                    tags: &["chat", "reasoning", "vision"],
                    supports_reasoning_effort: false,
                    default_max_tokens: Some(4096),
                    default_context_limit_tokens: Some(128_000),
                    default_reasoning_effort: None,
                    default_temperature_percent: Some(70),
                    default_timeout_ms: Some(60_000),
                },
                GatewayVendorModelPreset {
                    model: "claude-3.5-sonnet",
                    display_name: "Copilot Claude 3.5 Sonnet",
                    tags: &["chat", "vision"],
                    supports_reasoning_effort: false,
                    default_max_tokens: Some(4096),
                    default_context_limit_tokens: Some(128_000),
                    default_reasoning_effort: None,
                    default_temperature_percent: Some(70),
                    default_timeout_ms: Some(60_000),
                },
                GatewayVendorModelPreset {
                    model: "gpt-6-astra",
                    display_name: "Copilot GPT-6 Astra",
                    tags: &["chat", "reasoning", "vision"],
                    supports_reasoning_effort: true,
                    default_max_tokens: Some(128_000),
                    default_context_limit_tokens: Some(1_050_000),
                    default_reasoning_effort: Some("high"),
                    default_temperature_percent: None,
                    default_timeout_ms: Some(120_000),
                },
                GatewayVendorModelPreset {
                    model: "gpt-6.1-sol",
                    display_name: "Copilot GPT-6.1 Sol",
                    tags: &["chat", "reasoning", "vision"],
                    supports_reasoning_effort: true,
                    default_max_tokens: Some(128_000),
                    default_context_limit_tokens: Some(1_050_000),
                    default_reasoning_effort: Some("high"),
                    default_temperature_percent: None,
                    default_timeout_ms: Some(90_000),
                },
                GatewayVendorModelPreset {
                    model: "gpt-6.1-terra",
                    display_name: "Copilot GPT-6.1 Terra",
                    tags: &["chat", "reasoning", "vision"],
                    supports_reasoning_effort: true,
                    default_max_tokens: Some(128_000),
                    default_context_limit_tokens: Some(1_050_000),
                    default_reasoning_effort: Some("medium"),
                    default_temperature_percent: None,
                    default_timeout_ms: Some(90_000),
                },
                GatewayVendorModelPreset {
                    model: "gpt-6-sol",
                    display_name: "Copilot GPT-6 Sol",
                    tags: &["chat", "reasoning", "vision"],
                    supports_reasoning_effort: true,
                    default_max_tokens: Some(128_000),
                    default_context_limit_tokens: Some(1_050_000),
                    default_reasoning_effort: Some("high"),
                    default_temperature_percent: None,
                    default_timeout_ms: Some(90_000),
                },
                GatewayVendorModelPreset {
                    model: "gpt-6-luna",
                    display_name: "Copilot GPT-6 Luna",
                    tags: &["chat", "reasoning", "vision"],
                    supports_reasoning_effort: true,
                    default_max_tokens: Some(128_000),
                    default_context_limit_tokens: Some(1_050_000),
                    default_reasoning_effort: Some("low"),
                    default_temperature_percent: None,
                    default_timeout_ms: Some(60_000),
                },
                GatewayVendorModelPreset {
                    model: "gpt-5.6-sol",
                    display_name: "Copilot GPT-5.6 Sol",
                    tags: &["chat", "reasoning", "vision"],
                    supports_reasoning_effort: true,
                    default_max_tokens: Some(128_000),
                    default_context_limit_tokens: Some(1_050_000),
                    default_reasoning_effort: Some("high"),
                    default_temperature_percent: None,
                    default_timeout_ms: Some(90_000),
                },
                GatewayVendorModelPreset {
                    model: "gpt-5.6-terra",
                    display_name: "Copilot GPT-5.6 Terra",
                    tags: &["chat", "reasoning", "vision"],
                    supports_reasoning_effort: true,
                    default_max_tokens: Some(128_000),
                    default_context_limit_tokens: Some(1_050_000),
                    default_reasoning_effort: Some("high"),
                    default_temperature_percent: None,
                    default_timeout_ms: Some(90_000),
                },
                GatewayVendorModelPreset {
                    model: "gpt-5.6-luna",
                    display_name: "Copilot GPT-5.6 Luna",
                    tags: &["chat", "reasoning", "vision"],
                    supports_reasoning_effort: true,
                    default_max_tokens: Some(128_000),
                    default_context_limit_tokens: Some(1_050_000),
                    default_reasoning_effort: Some("low"),
                    default_temperature_percent: None,
                    default_timeout_ms: Some(60_000),
                },
                GatewayVendorModelPreset {
                    model: "gpt-5.4",
                    display_name: "Copilot GPT-5.4",
                    tags: &["chat", "reasoning", "vision"],
                    supports_reasoning_effort: true,
                    default_max_tokens: Some(128_000),
                    default_context_limit_tokens: Some(1_050_000),
                    default_reasoning_effort: Some("medium"),
                    default_temperature_percent: None,
                    default_timeout_ms: Some(90_000),
                },
                GatewayVendorModelPreset {
                    model: "gpt-5.3-codex",
                    display_name: "Copilot GPT-5.3 Codex",
                    tags: &["chat", "reasoning", "vision"],
                    supports_reasoning_effort: true,
                    default_max_tokens: Some(128_000),
                    default_context_limit_tokens: Some(400_000),
                    default_reasoning_effort: Some("high"),
                    default_temperature_percent: None,
                    default_timeout_ms: Some(90_000),
                },
                GatewayVendorModelPreset {
                    model: "gpt-5",
                    display_name: "Copilot GPT-5",
                    tags: &["chat", "reasoning", "vision"],
                    supports_reasoning_effort: true,
                    default_max_tokens: Some(128_000),
                    default_context_limit_tokens: Some(400_000),
                    default_reasoning_effort: Some("medium"),
                    default_temperature_percent: None,
                    default_timeout_ms: Some(90_000),
                },
                GatewayVendorModelPreset {
                    model: "gpt-5-mini",
                    display_name: "Copilot GPT-5 Mini",
                    tags: &["chat", "reasoning", "vision"],
                    supports_reasoning_effort: true,
                    default_max_tokens: Some(128_000),
                    default_context_limit_tokens: Some(400_000),
                    default_reasoning_effort: Some("low"),
                    default_temperature_percent: None,
                    default_timeout_ms: Some(60_000),
                },
                GatewayVendorModelPreset {
                    model: "gpt-4.1",
                    display_name: "Copilot GPT-4.1",
                    tags: &["chat", "vision"],
                    supports_reasoning_effort: false,
                    default_max_tokens: Some(4096),
                    default_context_limit_tokens: Some(128_000),
                    default_reasoning_effort: None,
                    default_temperature_percent: Some(70),
                    default_timeout_ms: Some(60_000),
                },
                GatewayVendorModelPreset {
                    model: "gpt-4o",
                    display_name: "Copilot GPT-4o",
                    tags: &["chat", "vision"],
                    supports_reasoning_effort: false,
                    default_max_tokens: Some(4096),
                    default_context_limit_tokens: Some(128_000),
                    default_reasoning_effort: None,
                    default_temperature_percent: Some(70),
                    default_timeout_ms: Some(60_000),
                },
                GatewayVendorModelPreset {
                    model: "o3-mini",
                    display_name: "Copilot o3-mini",
                    tags: &["chat", "reasoning"],
                    supports_reasoning_effort: true,
                    default_max_tokens: Some(4096),
                    default_context_limit_tokens: Some(128_000),
                    default_reasoning_effort: Some("medium"),
                    default_temperature_percent: None,
                    default_timeout_ms: Some(60_000),
                },
                GatewayVendorModelPreset {
                    model: "o4-mini",
                    display_name: "Copilot o4-mini",
                    tags: &["chat", "reasoning", "vision"],
                    supports_reasoning_effort: true,
                    default_max_tokens: Some(8192),
                    default_context_limit_tokens: Some(200_000),
                    default_reasoning_effort: Some("medium"),
                    default_temperature_percent: None,
                    default_timeout_ms: Some(60_000),
                },
                GatewayVendorModelPreset {
                    model: "gemini-2.5-pro",
                    display_name: "Copilot Gemini 2.5 Pro",
                    tags: &["chat", "reasoning", "vision"],
                    supports_reasoning_effort: true,
                    default_max_tokens: Some(8192),
                    default_context_limit_tokens: Some(1_000_000),
                    default_reasoning_effort: Some("medium"),
                    default_temperature_percent: Some(70),
                    default_timeout_ms: Some(90_000),
                },
                GatewayVendorModelPreset {
                    model: "gemini-2.0-flash-001",
                    display_name: "Copilot Gemini 2.0 Flash",
                    tags: &["chat", "vision"],
                    supports_reasoning_effort: false,
                    default_max_tokens: Some(4096),
                    default_context_limit_tokens: Some(128_000),
                    default_reasoning_effort: None,
                    default_temperature_percent: Some(70),
                    default_timeout_ms: Some(60_000),
                },
            ],
            Self::Muse => &[
                GatewayVendorModelPreset {
                    model: "muse-chat",
                    display_name: "Muse Chat (Multimodal)",
                    tags: &["chat", "vision"],
                    supports_reasoning_effort: false,
                    default_max_tokens: Some(4096),
                    default_context_limit_tokens: Some(128_000),
                    default_reasoning_effort: None,
                    default_temperature_percent: None,
                    default_timeout_ms: Some(90_000),
                },
                GatewayVendorModelPreset {
                    model: "muse-all",
                    display_name: "Muse All-in-One (Chat + Image + Video)",
                    tags: &["chat", "vision", "image", "video"],
                    supports_reasoning_effort: false,
                    default_max_tokens: Some(4096),
                    default_context_limit_tokens: Some(128_000),
                    default_reasoning_effort: None,
                    default_temperature_percent: None,
                    default_timeout_ms: Some(300_000),
                },
                GatewayVendorModelPreset {
                    model: "muse-image",
                    display_name: "Muse Image Generation",
                    tags: &["image"],
                    supports_reasoning_effort: false,
                    default_max_tokens: None,
                    default_context_limit_tokens: None,
                    default_reasoning_effort: None,
                    default_temperature_percent: None,
                    default_timeout_ms: Some(120_000),
                },
                GatewayVendorModelPreset {
                    model: "muse-video",
                    display_name: "Muse Video Generation",
                    tags: &["video"],
                    supports_reasoning_effort: false,
                    default_max_tokens: None,
                    default_context_limit_tokens: None,
                    default_reasoning_effort: None,
                    default_temperature_percent: None,
                    default_timeout_ms: Some(300_000),
                },
            ],
        }
    }
}

impl fmt::Display for GatewayVendor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Strongly-typed non-secret binding stored in `config_json.gateway_binding`
/// (`"account:<account_key>"` or `"pool:<vendor>"`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum GatewayBinding {
    Account(String),
    Pool(GatewayVendor),
}

impl GatewayBinding {
    pub fn parse(raw: &str) -> Result<Self, GatewayError> {
        let trimmed = raw.trim();
        if let Some(account_key) = trimmed.strip_prefix("account:") {
            let key = account_key.trim();
            if valid_account_key(key) {
                return Ok(Self::Account(key.to_owned()));
            }
            return Err(GatewayError::InvalidBinding);
        }
        if let Some(vendor_raw) = trimmed.strip_prefix("pool:") {
            let vendor = GatewayVendor::parse(vendor_raw).ok_or(GatewayError::InvalidBinding)?;
            return Ok(Self::Pool(vendor));
        }
        Err(GatewayError::InvalidBinding)
    }

    pub fn as_config_string(&self) -> String {
        match self {
            Self::Account(key) => format!("account:{key}"),
            Self::Pool(vendor) => format!("pool:{}", vendor.as_str()),
        }
    }
}

impl fmt::Display for GatewayBinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.as_config_string())
    }
}

impl FromStr for GatewayBinding {
    type Err = GatewayError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl TryFrom<String> for GatewayBinding {
    type Error = GatewayError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<GatewayBinding> for String {
    fn from(binding: GatewayBinding) -> Self {
        binding.as_config_string()
    }
}

pub fn valid_account_key(key: &str) -> bool {
    (3..=128).contains(&key.len())
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayAccountStatus {
    Active,
    ReauthRequired,
    Disabled,
}

impl GatewayAccountStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::ReauthRequired => "reauth_required",
            Self::Disabled => "disabled",
        }
    }

    pub fn parse(raw: &str) -> Self {
        match raw.trim() {
            "reauth_required" => Self::ReauthRequired,
            "disabled" => Self::Disabled,
            _ => Self::Active,
        }
    }
}

/// Encrypted payload sealed inside `llm_gateway_accounts.credential_json` via `SecretResolver::seal`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OAuthTokenBundle {
    pub access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// For GitHub Copilot: long-lived GitHub OAuth token used to mint short-lived Copilot bearer tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_oauth_token: Option<String>,
    /// Optional vendor-specific account/organization/project identifier (e.g. `ChatGPT-Account-Id`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_context_id: Option<String>,
}

impl OAuthTokenBundle {
    pub fn seal(
        &self,
        secret_resolver: &dyn SecretResolver,
        account_key: &str,
    ) -> Result<String, GatewayError> {
        if self.access_token.trim().is_empty() {
            return Err(GatewayError::InvalidTokenBundle);
        }
        let scope = format!("gateway_account:{account_key}");
        let json_bytes = serde_json::to_vec(self).map_err(|_| GatewayError::InvalidTokenBundle)?;
        let compact_b64 = URL_SAFE_NO_PAD.encode(&json_bytes);
        let secret_val = SecretValue::new(compact_b64);
        if !credentials::valid_input(&secret_val) {
            return Err(GatewayError::InvalidTokenBundle);
        }
        secret_resolver
            .seal(&secret_val, &scope)
            .map_err(|_| GatewayError::CredentialStorageUnavailable)
    }

    pub fn open(
        secret_resolver: &dyn SecretResolver,
        account_key: &str,
        credential_json: &str,
    ) -> Result<Self, GatewayError> {
        let scope = format!("gateway_account:{account_key}");
        let raw_ref = credentials::reference(&scope, Some(credential_json), None)
            .ok_or(GatewayError::CredentialInvalid)?;
        let secret_ref = SecretRef::parse(raw_ref).map_err(|_| GatewayError::CredentialInvalid)?;
        let resolved = secret_resolver
            .resolve(&secret_ref)
            .map_err(|_| GatewayError::CredentialInvalid)?;
        let json_bytes = URL_SAFE_NO_PAD
            .decode(resolved.expose().as_bytes())
            .map_err(|_| GatewayError::CredentialInvalid)?;
        let bundle: Self =
            serde_json::from_slice(&json_bytes).map_err(|_| GatewayError::CredentialInvalid)?;
        if bundle.access_token.trim().is_empty() {
            return Err(GatewayError::CredentialInvalid);
        }
        Ok(bundle)
    }
}

#[derive(Debug, Clone)]
pub struct GatewayLease {
    pub account_key: String,
    pub vendor: GatewayVendor,
    pub access_token: SecretString,
    pub base_url: String,
    pub extra_headers: Vec<(String, String)>,
    pub account_context_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayCallOutcome {
    Success,
    RateLimited { retry_after_secs: Option<u64> },
    Unauthorized,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OAuthLoginMode {
    Pkce,
    DeviceCode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct GatewayVendorModelPreset {
    pub model: &'static str,
    pub display_name: &'static str,
    pub tags: &'static [&'static str],
    pub supports_reasoning_effort: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_max_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_context_limit_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_reasoning_effort: Option<&'static str>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_temperature_percent: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayPoolModelView {
    #[serde(skip)]
    pub id: i64,
    pub provider_key: String,
    pub display_name: String,
    pub vendor: GatewayVendor,
    pub adapter: String,
    pub gateway_binding: GatewayBinding,
    pub binding_mode: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    pub enabled: bool,
    pub revision: i64,
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_limit_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature_percent: Option<u16>,
    pub timeout_ms: u64,
    pub template_usage_count: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayVendorInfo {
    pub vendor: GatewayVendor,
    pub display_name: String,
    pub login_mode: OAuthLoginMode,
    pub default_base_url: String,
    pub pool_binding: GatewayBinding,
    pub total_accounts: usize,
    pub active_accounts: usize,
    pub cooling_down_accounts: usize,
    #[serde(default)]
    pub model_count: usize,
    #[serde(default)]
    pub models: Vec<GatewayPoolModelView>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GatewayPoolDetailView {
    pub vendor: GatewayVendor,
    pub display_name: String,
    pub login_mode: OAuthLoginMode,
    pub default_base_url: String,
    pub pool_binding: GatewayBinding,
    pub total_accounts: usize,
    pub active_accounts: usize,
    pub cooling_down_accounts: usize,
    pub reauth_required_accounts: usize,
    pub disabled_accounts: usize,
    pub accounts: Vec<GatewayAccountView>,
    pub models: Vec<GatewayPoolModelView>,
    pub preset_models: &'static [GatewayVendorModelPreset],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GatewayQuotaWindow {
    pub name: String,
    pub used: f64,
    pub total: f64,
    pub remaining_percentage: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reset_at: Option<i64>,
    #[serde(default)]
    pub unlimited: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct VendorQuotaSnapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    #[serde(default)]
    pub limit_reached: bool,
    #[serde(default)]
    pub windows: Vec<GatewayQuotaWindow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Optional discovered upstream account/project context ID (e.g. Gemini `cloudaicompanionProject`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discovered_account_context_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GatewayAccountQuota {
    pub account_key: String,
    pub vendor: GatewayVendor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
    pub limit_reached: bool,
    pub windows: Vec<GatewayQuotaWindow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub fetched_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayBoundProviderRef {
    pub provider_key: String,
    pub display_name: String,
    pub binding_mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayAccountView {
    pub account_key: String,
    pub vendor: GatewayVendor,
    pub label: String,
    pub account_email: Option<String>,
    pub credential_metadata: Value,
    pub enabled: bool,
    pub priority: i64,
    pub status: GatewayAccountStatus,
    pub cooldown_remaining_secs: u64,
    pub expires_at: Option<i64>,
    pub last_refreshed_at: Option<i64>,
    pub last_used_at: Option<i64>,
    pub last_error: Option<String>,
    pub bound_providers: Vec<GatewayBoundProviderRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota: Option<GatewayAccountQuota>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StartOAuthLoginRequest {
    pub vendor: GatewayVendor,
    #[serde(alias = "display_name")]
    pub label: String,
    #[serde(default)]
    pub redirect_uri: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthLoginChallenge {
    pub session_id: String,
    pub vendor: GatewayVendor,
    pub mode: OAuthLoginMode,
    pub authorize_url: Option<String>,
    pub verification_uri: Option<String>,
    pub user_code: Option<String>,
    pub poll_interval_secs: u64,
    pub expires_in_secs: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OAuthCallbackSubmission {
    pub callback_url_or_code: String,
    #[serde(default)]
    pub state: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum OAuthPollStatus {
    Pending { poll_interval_secs: u64 },
    Completed { account: Box<GatewayAccountView> },
    Failed { error: String },
}

fn deserialize_double_option<'de, T, D>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct CreatePoolModelRequest {
    pub model: String,
    #[serde(default, alias = "name")]
    pub display_name: Option<String>,
    #[serde(default)]
    pub gateway_binding: Option<GatewayBinding>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub context_limit_tokens: Option<u32>,
    #[serde(default)]
    pub temperature_percent: Option<u16>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct UpdatePoolModelPatch {
    pub model: Option<String>,
    #[serde(default, alias = "name")]
    pub display_name: Option<String>,
    pub gateway_binding: Option<GatewayBinding>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub base_url: Option<Option<String>>,
    pub tags: Option<Vec<String>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub reasoning_effort: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub max_tokens: Option<Option<u32>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub context_limit_tokens: Option<Option<u32>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub temperature_percent: Option<Option<u16>>,
    pub timeout_ms: Option<u64>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct UpdateGatewayAccountPatch {
    #[serde(alias = "display_name")]
    pub label: Option<String>,
    pub enabled: Option<bool>,
    pub priority: Option<i64>,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum GatewayError {
    #[error("invalid_gateway_binding")]
    InvalidBinding,
    #[error("invalid_token_bundle")]
    InvalidTokenBundle,
    #[error("credential_storage_unavailable")]
    CredentialStorageUnavailable,
    #[error("credential_invalid")]
    CredentialInvalid,
    #[error("gateway_account_not_found")]
    AccountNotFound,
    #[error("gateway_account_disabled")]
    AccountDisabled,
    #[error("gateway_account_cooling_down")]
    AccountCoolingDown,
    #[error("gateway_account_reauth_required")]
    AccountReauthRequired,
    #[error("gateway_pool_exhausted")]
    PoolExhausted,
    #[error("oauth_session_not_found")]
    OAuthSessionNotFound,
    #[error("oauth_session_expired")]
    OAuthSessionExpired,
    #[error("oauth_state_mismatch")]
    OAuthStateMismatch,
    #[error("oauth_callback_invalid")]
    OAuthCallbackInvalid,
    #[error("oauth_exchange_failed: {0}")]
    OAuthExchangeFailed(String),
    #[error("gateway_account_in_use")]
    AccountInUse(Vec<String>),
    #[error("gateway_pool_model_not_found")]
    PoolModelNotFound,
    #[error("gateway_pool_model_in_use")]
    PoolModelInUse(Vec<String>),
    #[error("database_unavailable")]
    DatabaseUnavailable,
    #[error("validation_failed")]
    ValidationFailed,
}

impl GatewayError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidBinding => "invalid_gateway_binding",
            Self::InvalidTokenBundle => "invalid_token_bundle",
            Self::CredentialStorageUnavailable => "credential_storage_unavailable",
            Self::CredentialInvalid => "credential_invalid",
            Self::AccountNotFound => "gateway_account_not_found",
            Self::AccountDisabled => "gateway_account_disabled",
            Self::AccountCoolingDown => "gateway_account_cooling_down",
            Self::AccountReauthRequired => "gateway_account_reauth_required",
            Self::PoolExhausted => "gateway_pool_exhausted",
            Self::OAuthSessionNotFound => "oauth_session_not_found",
            Self::OAuthSessionExpired => "oauth_session_expired",
            Self::OAuthStateMismatch => "oauth_state_mismatch",
            Self::OAuthCallbackInvalid => "oauth_callback_invalid",
            Self::OAuthExchangeFailed(_) => "oauth_exchange_failed",
            Self::AccountInUse(_) => "gateway_account_in_use",
            Self::PoolModelNotFound => "gateway_pool_model_not_found",
            Self::PoolModelInUse(_) => "gateway_pool_model_in_use",
            Self::DatabaseUnavailable => "database_unavailable",
            Self::ValidationFailed => "validation_failed",
        }
    }
}
