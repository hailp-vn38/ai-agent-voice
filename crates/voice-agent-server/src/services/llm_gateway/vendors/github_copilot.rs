use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;

use super::{DeviceCodeGrant, DevicePollOutcome, ExchangedOAuthAccount, OAuthVendorDriver};
use crate::services::llm_gateway::types::{GatewayError, GatewayVendor, OAuthTokenBundle};

const COPILOT_CLIENT_ID: &str = "Iv1.b507a08c87ecfe98";
const GITHUB_DEVICE_CODE_URL: &str = "https://github.com/login/device/code";
const GITHUB_ACCESS_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
const COPILOT_TOKEN_EXCHANGE_URL: &str = "https://api.github.com/copilot_internal/v2/token";

pub struct GithubCopilotOAuthDriver {
    http: reqwest::Client,
}

impl GithubCopilotOAuthDriver {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .user_agent("GitHubCopilotChat/0.24.0")
            .build()
            .unwrap_or_default();
        Self { http }
    }

    async fn exchange_for_copilot_bearer(
        &self,
        github_oauth_token: &str,
    ) -> Result<(String, Option<i64>), GatewayError> {
        #[derive(Deserialize)]
        struct CopilotTokenResp {
            token: String,
            expires_at: Option<i64>,
        }
        let resp = self
            .http
            .get(COPILOT_TOKEN_EXCHANGE_URL)
            .header("Authorization", format!("token {github_oauth_token}"))
            .header("Accept", "application/json")
            .header("Editor-Version", "vscode/1.96.0")
            .header("Copilot-Integration-Id", "vscode-chat")
            .send()
            .await
            .map_err(|e| GatewayError::OAuthExchangeFailed(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(GatewayError::AccountReauthRequired);
        }
        let parsed: CopilotTokenResp = resp
            .json()
            .await
            .map_err(|e| GatewayError::OAuthExchangeFailed(e.to_string()))?;
        Ok((parsed.token, parsed.expires_at))
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

#[async_trait]
impl OAuthVendorDriver for GithubCopilotOAuthDriver {
    fn vendor(&self) -> GatewayVendor {
        GatewayVendor::GithubCopilot
    }

    async fn start_device_flow(&self) -> Result<DeviceCodeGrant, GatewayError> {
        let body = serde_json::json!({
            "client_id": COPILOT_CLIENT_ID,
            "scope": "read:user",
        });
        let resp = self
            .http
            .post(GITHUB_DEVICE_CODE_URL)
            .header("Accept", "application/json")
            .json(&body)
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
        let body = serde_json::json!({
            "client_id": COPILOT_CLIENT_ID,
            "device_code": device_code,
            "grant_type": "urn:ietf:params:oauth:grant-type:device_code",
        });
        let resp = self
            .http
            .post(GITHUB_ACCESS_TOKEN_URL)
            .header("Accept", "application/json")
            .json(&body)
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
            let (copilot_bearer, expires_at) = self
                .exchange_for_copilot_bearer(&github_oauth_token)
                .await?;
            return Ok(DevicePollOutcome::Completed(ExchangedOAuthAccount {
                bundle: OAuthTokenBundle {
                    access_token: copilot_bearer,
                    refresh_token: None,
                    upstream_oauth_token: Some(github_oauth_token),
                    account_context_id: None,
                },
                account_email: None,
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

    fn extra_headers_for(&self, _bundle: &OAuthTokenBundle) -> Vec<(String, String)> {
        vec![
            ("Editor-Version".to_owned(), "vscode/1.96.0".to_owned()),
            ("Copilot-Integration-Id".to_owned(), "vscode-chat".to_owned()),
        ]
    }
}
