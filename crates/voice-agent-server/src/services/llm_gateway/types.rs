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
}

impl GatewayVendor {
    pub const ALL: [Self; 4] = [
        Self::Anthropic,
        Self::OpenaiCodex,
        Self::Gemini,
        Self::GithubCopilot,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::OpenaiCodex => "openai_codex",
            Self::Gemini => "gemini",
            Self::GithubCopilot => "github_copilot",
        }
    }

    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Anthropic => "Anthropic Claude",
            Self::OpenaiCodex => "OpenAI Codex (ChatGPT)",
            Self::Gemini => "Google Gemini",
            Self::GithubCopilot => "GitHub Copilot",
        }
    }

    pub const fn default_base_url(self) -> &'static str {
        match self {
            Self::Anthropic => "https://api.anthropic.com/v1",
            Self::OpenaiCodex => "https://api.openai.com/v1",
            Self::Gemini => "https://generativelanguage.googleapis.com/v1beta/openai",
            Self::GithubCopilot => "https://api.githubcopilot.com",
        }
    }

    pub const fn login_mode(self) -> OAuthLoginMode {
        match self {
            Self::Anthropic | Self::OpenaiCodex | Self::Gemini => OAuthLoginMode::Pkce,
            Self::GithubCopilot => OAuthLoginMode::DeviceCode,
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "anthropic" => Some(Self::Anthropic),
            "openai_codex" => Some(Self::OpenaiCodex),
            "gemini" => Some(Self::Gemini),
            "github_copilot" => Some(Self::GithubCopilot),
            _ => None,
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
        let json_bytes =
            serde_json::to_vec(self).map_err(|_| GatewayError::InvalidTokenBundle)?;
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
        let secret_ref =
            SecretRef::parse(raw_ref).map_err(|_| GatewayError::CredentialInvalid)?;
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayVendorInfo {
    pub vendor: GatewayVendor,
    pub display_name: String,
    pub login_mode: OAuthLoginMode,
    pub default_base_url: String,
    pub pool_binding: String,
    pub total_accounts: usize,
    pub active_accounts: usize,
    pub cooling_down_accounts: usize,
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
    Completed { account: GatewayAccountView },
    Failed { error: String },
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
            Self::DatabaseUnavailable => "database_unavailable",
            Self::ValidationFailed => "validation_failed",
        }
    }
}
