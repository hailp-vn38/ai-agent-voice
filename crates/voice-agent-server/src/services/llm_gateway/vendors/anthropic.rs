use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use url::Url;

use super::{ExchangedOAuthAccount, OAuthVendorDriver, parse_reset_timestamp};
use crate::services::llm_gateway::types::{
    GatewayError, GatewayQuotaWindow, GatewayVendor, OAuthTokenBundle, VendorQuotaSnapshot,
};

const ANTHROPIC_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const ANTHROPIC_AUTHORIZE_URL: &str = "https://claude.ai/oauth/authorize";
const ANTHROPIC_TOKEN_URL: &str = "https://api.anthropic.com/v1/oauth/token";
const ANTHROPIC_USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage?cedar_ember=1";
const ANTHROPIC_REDIRECT_URI: &str = "https://console.anthropic.com/oauth/code/callback";
const ANTHROPIC_SCOPES: &str = "org:create_api_key user:profile user:inference";
const ANTHROPIC_BETA_HEADER: &str =
    "claude-code-20250219,oauth-2025-04-20,interleaved-thinking-2025-05-14";
const ANTHROPIC_CLI_USER_AGENT: &str = "claude-cli/2.1.63 (external, cli)";

pub struct AnthropicOAuthDriver {
    http: reqwest::Client,
}

impl AnthropicOAuthDriver {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .unwrap_or_default();
        Self { http }
    }
}

#[derive(Deserialize)]
struct AnthropicTokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
    #[serde(default)]
    account: Option<AnthropicAccountInfo>,
}

#[derive(Deserialize)]
struct AnthropicAccountInfo {
    email_address: Option<String>,
}

fn parse_utilization_window(name: String, node: Option<&Value>) -> Option<GatewayQuotaWindow> {
    let obj = node?;
    let utilization = obj
        .get("utilization")
        .and_then(Value::as_f64)
        .or_else(|| obj.get("percent").and_then(Value::as_f64))?;
    let used = utilization.clamp(0.0, 100.0);
    let remaining_percentage = (100.0 - used).clamp(0.0, 100.0);
    let reset_at = parse_reset_timestamp(
        obj.get("resets_at")
            .or_else(|| obj.get("reset_at"))
            .or_else(|| obj.get("resetsAt")),
    );
    Some(GatewayQuotaWindow {
        name,
        used,
        total: 100.0,
        remaining_percentage,
        reset_at,
        unlimited: false,
        unit: Some("%".to_owned()),
    })
}

pub(crate) fn parse_anthropic_usage_payload(data: &Value) -> VendorQuotaSnapshot {
    let mut windows = Vec::new();
    let mut limit_reached = false;

    if let Some(w) = parse_utilization_window("Session (5h)".to_owned(), data.get("five_hour")) {
        if w.remaining_percentage <= 0.0 {
            limit_reached = true;
        }
        windows.push(w);
    }
    if let Some(w) = parse_utilization_window("Weekly (7d)".to_owned(), data.get("seven_day")) {
        if w.remaining_percentage <= 0.0 {
            limit_reached = true;
        }
        windows.push(w);
    }

    if let Some(obj) = data.as_object() {
        for (key, val) in obj {
            if let Some(suffix) = key.strip_prefix("seven_day_")
                && !suffix.is_empty()
                && val.is_object()
            {
                let label = suffix
                    .split('_')
                    .filter(|s| !s.is_empty())
                    .map(|word| {
                        let mut chars = word.chars();
                        match chars.next() {
                            Some(first) => {
                                format!("{}{}", first.to_ascii_uppercase(), chars.as_str())
                            }
                            None => String::new(),
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                if let Some(w) =
                    parse_utilization_window(format!("Weekly {label} (7d)"), Some(val))
                {
                    windows.push(w);
                }
            }
        }
    }

    if let Some(limits) = data.get("limits").and_then(Value::as_array) {
        for limit in limits {
            if limit.get("kind").and_then(Value::as_str) == Some("weekly_scoped") {
                let model_name = limit
                    .get("scope")
                    .and_then(|s| s.get("model"))
                    .and_then(|m| {
                        m.get("display_name")
                            .and_then(Value::as_str)
                            .or_else(|| m.get("id").and_then(Value::as_str))
                    })
                    .unwrap_or("Model");
                if let Some(w) = parse_utilization_window(
                    format!("Weekly {model_name} (7d)"),
                    Some(limit),
                ) {
                    windows.push(w);
                }
            }
        }
    }

    VendorQuotaSnapshot {
        plan: Some("Claude Code".to_owned()),
        limit_reached,
        windows,
        message: None,
        discovered_account_context_id: None,
    }
}

#[async_trait]
impl OAuthVendorDriver for AnthropicOAuthDriver {
    fn vendor(&self) -> GatewayVendor {
        GatewayVendor::Anthropic
    }

    fn default_redirect_uri(&self) -> &'static str {
        ANTHROPIC_REDIRECT_URI
    }

    fn build_pkce_authorize_url(
        &self,
        state: &str,
        code_challenge: &str,
        redirect_uri: &str,
    ) -> Result<String, GatewayError> {
        let mut url =
            Url::parse(ANTHROPIC_AUTHORIZE_URL).map_err(|_| GatewayError::OAuthCallbackInvalid)?;
        url.query_pairs_mut()
            .append_pair("code", "true")
            .append_pair("client_id", ANTHROPIC_CLIENT_ID)
            .append_pair("response_type", "code")
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("scope", ANTHROPIC_SCOPES)
            .append_pair("code_challenge", code_challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", state);
        Ok(url.to_string())
    }

    async fn exchange_pkce_code(
        &self,
        code: &str,
        state: &str,
        code_verifier: &str,
        redirect_uri: &str,
    ) -> Result<ExchangedOAuthAccount, GatewayError> {
        let (auth_code, resolved_state) = match code.split_once('#') {
            Some((c, s)) if !s.trim().is_empty() => (c.trim(), s.trim()),
            Some((c, _)) => (c.trim(), state),
            None => (code.trim(), state),
        };
        let body = json!({
            "grant_type": "authorization_code",
            "client_id": ANTHROPIC_CLIENT_ID,
            "code": auth_code,
            "state": resolved_state,
            "redirect_uri": redirect_uri,
            "code_verifier": code_verifier,
        });
        let resp = self
            .http
            .post(ANTHROPIC_TOKEN_URL)
            .header("Accept", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| GatewayError::OAuthExchangeFailed(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(GatewayError::OAuthExchangeFailed(format!(
                "anthropic_token_http_{}",
                resp.status().as_u16()
            )));
        }
        let parsed: AnthropicTokenResponse = resp
            .json()
            .await
            .map_err(|e| GatewayError::OAuthExchangeFailed(e.to_string()))?;
        let now = crate::database::unix_seconds().unwrap_or(0);
        let expires_at = parsed.expires_in.map(|ttl| now + ttl.max(60));
        Ok(ExchangedOAuthAccount {
            bundle: OAuthTokenBundle {
                access_token: parsed.access_token,
                refresh_token: parsed.refresh_token,
                upstream_oauth_token: None,
                account_context_id: None,
            },
            account_email: parsed.account.and_then(|a| a.email_address),
            expires_at,
        })
    }

    async fn refresh_token(
        &self,
        current: &OAuthTokenBundle,
    ) -> Result<ExchangedOAuthAccount, GatewayError> {
        let refresh_token = current
            .refresh_token
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or(GatewayError::AccountReauthRequired)?;
        let body = json!({
            "grant_type": "refresh_token",
            "client_id": ANTHROPIC_CLIENT_ID,
            "refresh_token": refresh_token,
        });
        let resp = self
            .http
            .post(ANTHROPIC_TOKEN_URL)
            .header("Accept", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| GatewayError::OAuthExchangeFailed(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(GatewayError::AccountReauthRequired);
        }
        let parsed: AnthropicTokenResponse = resp
            .json()
            .await
            .map_err(|e| GatewayError::OAuthExchangeFailed(e.to_string()))?;
        let now = crate::database::unix_seconds().unwrap_or(0);
        let expires_at = parsed.expires_in.map(|ttl| now + ttl.max(60));
        Ok(ExchangedOAuthAccount {
            bundle: OAuthTokenBundle {
                access_token: parsed.access_token,
                refresh_token: parsed
                    .refresh_token
                    .or_else(|| current.refresh_token.clone()),
                upstream_oauth_token: None,
                account_context_id: current.account_context_id.clone(),
            },
            account_email: parsed.account.and_then(|a| a.email_address),
            expires_at,
        })
    }

    async fn fetch_quota(
        &self,
        bundle: &OAuthTokenBundle,
    ) -> Result<VendorQuotaSnapshot, GatewayError> {
        let resp = self
            .http
            .get(ANTHROPIC_USAGE_URL)
            .bearer_auth(&bundle.access_token)
            .header("Accept", "application/json")
            .header("anthropic-version", "2023-06-01")
            .header("anthropic-beta", "oauth-2025-04-20")
            .header("User-Agent", ANTHROPIC_CLI_USER_AGENT)
            .send()
            .await
            .map_err(|e| {
                GatewayError::OAuthExchangeFailed(format!("anthropic_quota_error: {e}"))
            })?;
        if resp.status().as_u16() == 401 {
            return Err(GatewayError::AccountReauthRequired);
        }
        if !resp.status().is_success() {
            return Err(GatewayError::OAuthExchangeFailed(format!(
                "anthropic_quota_http_{}",
                resp.status().as_u16()
            )));
        }
        let payload: Value = resp
            .json()
            .await
            .map_err(|e| GatewayError::OAuthExchangeFailed(format!("anthropic_quota_json: {e}")))?;
        Ok(parse_anthropic_usage_payload(&payload))
    }

    fn extra_headers_for(&self, _bundle: &OAuthTokenBundle) -> Vec<(String, String)> {
        vec![
            ("anthropic-version".to_owned(), "2023-06-01".to_owned()),
            (
                "anthropic-beta".to_owned(),
                ANTHROPIC_BETA_HEADER.to_owned(),
            ),
        ]
    }
}
