use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

use super::{
    DeviceCodeGrant, DevicePollOutcome, ExchangedOAuthAccount, OAuthVendorDriver,
    parse_reset_timestamp,
};
use crate::services::llm_gateway::types::{
    GatewayError, GatewayQuotaWindow, GatewayVendor, OAuthTokenBundle, VendorQuotaSnapshot,
};

const COPILOT_CLIENT_ID: &str = "Iv1.b507a08c87ecfe98";
const GITHUB_DEVICE_CODE_URL: &str = "https://github.com/login/device/code";
const GITHUB_ACCESS_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
const COPILOT_TOKEN_EXCHANGE_URL: &str = "https://api.github.com/copilot_internal/v2/token";
const COPILOT_USAGE_URL: &str = "https://api.github.com/copilot_internal/user";
const GITHUB_USERINFO_URL: &str = "https://api.github.com/user";
const GITHUB_API_VERSION: &str = "2022-11-28";
const COPILOT_USER_AGENT: &str = "GitHubCopilotChat/0.26.7";
const COPILOT_EDITOR_VERSION: &str = "vscode/1.110.0";
const COPILOT_PLUGIN_VERSION: &str = "copilot-chat/0.26.7";

pub struct GithubCopilotOAuthDriver {
    http: reqwest::Client,
    device_code_url: String,
    access_token_url: String,
    copilot_token_exchange_url: String,
    userinfo_url: String,
    usage_url: String,
}

impl GithubCopilotOAuthDriver {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .user_agent(COPILOT_USER_AGENT)
            .build()
            .unwrap_or_default();
        Self {
            http,
            device_code_url: GITHUB_DEVICE_CODE_URL.to_owned(),
            access_token_url: GITHUB_ACCESS_TOKEN_URL.to_owned(),
            copilot_token_exchange_url: COPILOT_TOKEN_EXCHANGE_URL.to_owned(),
            userinfo_url: GITHUB_USERINFO_URL.to_owned(),
            usage_url: COPILOT_USAGE_URL.to_owned(),
        }
    }

    #[cfg(test)]
    pub fn with_test_endpoints(base_url: &str) -> Self {
        let base = base_url.trim_end_matches('/');
        let mut drv = Self::new();
        drv.device_code_url = format!("{base}/login/device/code");
        drv.access_token_url = format!("{base}/login/oauth/access_token");
        drv.copilot_token_exchange_url = format!("{base}/copilot_internal/v2/token");
        drv.userinfo_url = format!("{base}/user");
        drv.usage_url = format!("{base}/copilot_internal/user");
        drv
    }

    async fn exchange_for_copilot_bearer(
        &self,
        github_oauth_token: &str,
    ) -> Result<(String, Option<i64>), GatewayError> {
        let resp = self
            .http
            .get(&self.copilot_token_exchange_url)
            .header("Authorization", format!("token {github_oauth_token}"))
            .header("Accept", "application/json")
            .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
            .header("User-Agent", COPILOT_USER_AGENT)
            .header("Editor-Version", COPILOT_EDITOR_VERSION)
            .header("Editor-Plugin-Version", COPILOT_PLUGIN_VERSION)
            .header("Copilot-Integration-Id", "vscode-chat")
            .send()
            .await
            .map_err(|e| GatewayError::OAuthExchangeFailed(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(GatewayError::AccountReauthRequired);
        }
        let payload: Value = resp
            .json()
            .await
            .map_err(|e| GatewayError::OAuthExchangeFailed(e.to_string()))?;
        let token = payload
            .get("token")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                GatewayError::OAuthExchangeFailed("missing_copilot_bearer_token".to_owned())
            })?
            .to_owned();
        let expires_at = parse_reset_timestamp(payload.get("expires_at"));
        Ok((token, expires_at))
    }

    async fn fetch_github_account_identity(&self, github_oauth_token: &str) -> Option<String> {
        #[derive(Deserialize)]
        struct GithubUserInfo {
            email: Option<String>,
            login: Option<String>,
            name: Option<String>,
        }
        let resp = self
            .http
            .get(&self.userinfo_url)
            .header("Authorization", format!("Bearer {github_oauth_token}"))
            .header("Accept", "application/json")
            .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
            .header("User-Agent", COPILOT_USER_AGENT)
            .send()
            .await
            .ok()?;
        if !resp.status().is_success() {
            return None;
        }
        let info: GithubUserInfo = resp.json().await.ok()?;
        info.email
            .filter(|s| !s.trim().is_empty())
            .or_else(|| info.login.filter(|s| !s.trim().is_empty()))
            .or_else(|| info.name.filter(|s| !s.trim().is_empty()))
    }
}

#[derive(Deserialize)]
struct GithubDeviceCodeResp {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    interval: Option<u64>,
}

#[derive(Deserialize)]
struct GithubPollTokenResp {
    access_token: Option<String>,
    error: Option<String>,
    interval: Option<u64>,
}

pub(crate) fn parse_github_copilot_usage_payload(data: &Value) -> VendorQuotaSnapshot {
    let mut windows = Vec::new();
    let mut limit_reached = false;

    if let Some(snapshots) = data.get("quota_snapshots").and_then(Value::as_object) {
        let plan = data
            .get("copilot_plan")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(str::to_owned);
        let reset_at = parse_reset_timestamp(data.get("quota_reset_date"));

        for (key, display_name) in [
            ("chat", "Chat"),
            ("completions", "Completions"),
            ("premium_interactions", "Premium Requests"),
        ] {
            let Some(snap) = snapshots.get(key) else {
                continue;
            };
            let unlimited = snap
                .get("unlimited")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let total = snap
                .get("entitlement")
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
            let remaining = snap.get("remaining").and_then(Value::as_f64).unwrap_or(0.0);
            let used = if unlimited {
                0.0
            } else {
                (total - remaining).max(0.0)
            };
            let remaining_percentage = if unlimited {
                100.0
            } else if let Some(pct) = snap.get("percent_remaining").and_then(Value::as_f64) {
                pct.clamp(0.0, 100.0)
            } else if total > 0.0 {
                (remaining / total * 100.0).clamp(0.0, 100.0)
            } else {
                0.0
            };
            if !unlimited && total > 0.0 && remaining <= 0.0 {
                limit_reached = true;
            }
            windows.push(GatewayQuotaWindow {
                name: display_name.to_owned(),
                used,
                total,
                remaining_percentage,
                reset_at,
                unlimited,
                unit: Some("req".to_owned()),
            });
        }
        return VendorQuotaSnapshot {
            plan,
            limit_reached,
            windows,
            message: None,
            discovered_account_context_id: None,
        };
    }

    let plan = data
        .get("copilot_plan")
        .and_then(Value::as_str)
        .or_else(|| data.get("access_type_sku").and_then(Value::as_str))
        .filter(|s| !s.trim().is_empty())
        .map(str::to_owned);
    let reset_at = parse_reset_timestamp(
        data.get("limited_user_reset_date")
            .or_else(|| data.get("quota_reset_date")),
    );
    let monthly = data.get("monthly_quotas");
    let limited = data.get("limited_user_quotas");

    if monthly.is_some() || limited.is_some() {
        for (key, display_name) in [("chat", "Chat"), ("completions", "Completions")] {
            let total = monthly
                .and_then(|m| m.get(key))
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
            let remaining = limited
                .and_then(|l| l.get(key))
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
            if total <= 0.0 && remaining <= 0.0 {
                continue;
            }
            let used = (total - remaining).max(0.0);
            let remaining_percentage = if total > 0.0 {
                (remaining / total * 100.0).clamp(0.0, 100.0)
            } else {
                0.0
            };
            if total > 0.0 && remaining <= 0.0 {
                limit_reached = true;
            }
            windows.push(GatewayQuotaWindow {
                name: display_name.to_owned(),
                used,
                total,
                remaining_percentage,
                reset_at,
                unlimited: false,
                unit: Some("req".to_owned()),
            });
        }
    }

    VendorQuotaSnapshot {
        plan,
        limit_reached,
        windows,
        message: None,
        discovered_account_context_id: None,
    }
}

#[async_trait]
impl OAuthVendorDriver for GithubCopilotOAuthDriver {
    fn vendor(&self) -> GatewayVendor {
        GatewayVendor::GithubCopilot
    }

    async fn start_device_flow(&self) -> Result<DeviceCodeGrant, GatewayError> {
        let form_body = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("client_id", COPILOT_CLIENT_ID)
            .append_pair("scope", "read:user")
            .finish();
        let resp = self
            .http
            .post(&self.device_code_url)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("Accept", "application/json")
            .body(form_body)
            .send()
            .await
            .map_err(|e| GatewayError::OAuthExchangeFailed(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(GatewayError::OAuthExchangeFailed(format!(
                "github_device_http_{}",
                resp.status().as_u16()
            )));
        }
        let parsed: GithubDeviceCodeResp = resp
            .json()
            .await
            .map_err(|e| GatewayError::OAuthExchangeFailed(e.to_string()))?;
        Ok(DeviceCodeGrant {
            device_code: parsed.device_code,
            user_code: parsed.user_code,
            verification_uri: parsed.verification_uri,
            interval_secs: parsed.interval.unwrap_or(5).max(5),
            expires_in_secs: parsed.expires_in,
        })
    }

    async fn poll_device_flow(&self, device_code: &str) -> Result<DevicePollOutcome, GatewayError> {
        let form_body = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("client_id", COPILOT_CLIENT_ID)
            .append_pair("device_code", device_code)
            .append_pair("grant_type", "urn:ietf:params:oauth:grant-type:device_code")
            .finish();
        let resp = self
            .http
            .post(&self.access_token_url)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("Accept", "application/json")
            .body(form_body)
            .send()
            .await
            .map_err(|e| GatewayError::OAuthExchangeFailed(e.to_string()))?;
        if !resp.status().is_success() {
            return Ok(DevicePollOutcome::Failed(format!(
                "github_poll_http_{}",
                resp.status().as_u16()
            )));
        }
        let parsed: GithubPollTokenResp = resp
            .json()
            .await
            .map_err(|e| GatewayError::OAuthExchangeFailed(e.to_string()))?;
        if let Some(github_oauth_token) = parsed.access_token.filter(|s: &String| !s.is_empty()) {
            // Match 9router's `postExchange`: minting the short-lived Copilot bearer token is
            // best-effort during initial device-code completion so a transient error on
            // `copilot_internal/v2/token` never discards the one-shot GitHub OAuth token.
            let (copilot_bearer, expires_at) =
                match self.exchange_for_copilot_bearer(&github_oauth_token).await {
                    Ok(pair) => pair,
                    Err(_) => {
                        let fallback_exp = crate::database::unix_seconds().map(|now| now + 60);
                        (github_oauth_token.clone(), fallback_exp)
                    }
                };
            let account_email = self
                .fetch_github_account_identity(&github_oauth_token)
                .await;
            return Ok(DevicePollOutcome::Completed(ExchangedOAuthAccount {
                bundle: OAuthTokenBundle {
                    access_token: copilot_bearer,
                    refresh_token: None,
                    upstream_oauth_token: Some(github_oauth_token),
                    account_context_id: None,
                },
                account_email,
                expires_at,
            }));
        }
        match parsed.error.as_deref() {
            Some("authorization_pending") | None => Ok(DevicePollOutcome::Pending {
                next_interval_secs: parsed.interval,
            }),
            Some("slow_down") => Ok(DevicePollOutcome::Pending {
                next_interval_secs: Some(parsed.interval.unwrap_or(10).max(10)),
            }),
            Some(err) => Ok(DevicePollOutcome::Failed(err.to_owned())),
        }
    }

    async fn refresh_token(
        &self,
        current: &OAuthTokenBundle,
    ) -> Result<ExchangedOAuthAccount, GatewayError> {
        let github_oauth_token = current
            .upstream_oauth_token
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or(GatewayError::AccountReauthRequired)?;
        let (copilot_bearer, expires_at) =
            self.exchange_for_copilot_bearer(github_oauth_token).await?;
        Ok(ExchangedOAuthAccount {
            bundle: OAuthTokenBundle {
                access_token: copilot_bearer,
                refresh_token: None,
                upstream_oauth_token: Some(github_oauth_token.to_owned()),
                account_context_id: None,
            },
            account_email: None,
            expires_at,
        })
    }

    async fn fetch_quota(
        &self,
        bundle: &OAuthTokenBundle,
    ) -> Result<VendorQuotaSnapshot, GatewayError> {
        let github_token = bundle
            .upstream_oauth_token
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(&bundle.access_token);
        let resp = self
            .http
            .get(&self.usage_url)
            .header("Authorization", format!("token {github_token}"))
            .header("Accept", "application/json")
            .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
            .header("Editor-Version", COPILOT_EDITOR_VERSION)
            .header("Editor-Plugin-Version", COPILOT_PLUGIN_VERSION)
            .header("User-Agent", COPILOT_USER_AGENT)
            .send()
            .await
            .map_err(|e| GatewayError::OAuthExchangeFailed(format!("copilot_quota_error: {e}")))?;
        if resp.status().as_u16() == 401 {
            return Err(GatewayError::AccountReauthRequired);
        }
        if !resp.status().is_success() {
            return Err(GatewayError::OAuthExchangeFailed(format!(
                "copilot_quota_http_{}",
                resp.status().as_u16()
            )));
        }
        let payload: Value = resp
            .json()
            .await
            .map_err(|e| GatewayError::OAuthExchangeFailed(format!("copilot_quota_json: {e}")))?;
        Ok(parse_github_copilot_usage_payload(&payload))
    }

    /// Headers for the Copilot inference API (`api.githubcopilot.com`). The GitHub REST
    /// `X-GitHub-Api-Version` (`2022-11-28`) belongs only to `api.github.com`; the Copilot
    /// inference API rejects it with `400 bad request: error: invalid apiVersion`.
    fn extra_headers_for(&self, _bundle: &OAuthTokenBundle) -> Vec<(String, String)> {
        vec![
            (
                "Editor-Version".to_owned(),
                COPILOT_EDITOR_VERSION.to_owned(),
            ),
            (
                "Editor-Plugin-Version".to_owned(),
                COPILOT_PLUGIN_VERSION.to_owned(),
            ),
            (
                "Copilot-Integration-Id".to_owned(),
                "vscode-chat".to_owned(),
            ),
            ("User-Agent".to_owned(), COPILOT_USER_AGENT.to_owned()),
        ]
    }
}
