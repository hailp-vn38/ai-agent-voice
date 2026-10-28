use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;

use super::{DeviceCodeGrant, DevicePollOutcome, ExchangedOAuthAccount, OAuthVendorDriver};
use crate::services::llm_gateway::types::{GatewayError, GatewayVendor, OAuthTokenBundle};

const COPILOT_CLIENT_ID: &str = "Iv1.b507a08c87ecfe98";
const GITHUB_DEVICE_CODE_URL: &str = "https://github.com/login/device/code";
const GITHUB_ACCESS_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";
const COPILOT_TOKEN_EXCHANGE_URL: &str = "https://api.github.com/copilot_internal/v2/token";
const GITHUB_USERINFO_URL: &str = "https://api.github.com/user";
const GITHUB_API_VERSION: &str = "2022-11-28";
const COPILOT_USER_AGENT: &str = "GitHubCopilotChat/0.26.7";
const COPILOT_EDITOR_VERSION: &str = "vscode/1.110.0";
const COPILOT_PLUGIN_VERSION: &str = "copilot-chat/0.26.7";

pub struct GithubCopilotOAuthDriver {
    http: reqwest::Client,
}

impl GithubCopilotOAuthDriver {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .user_agent(COPILOT_USER_AGENT)
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
            .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
            .header("Editor-Version", COPILOT_EDITOR_VERSION)
            .header("Editor-Plugin-Version", COPILOT_PLUGIN_VERSION)
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

    async fn fetch_github_account_identity(&self, github_oauth_token: &str) -> Option<String> {
        #[derive(Deserialize)]
        struct GithubUserInfo {
            email: Option<String>,
            login: Option<String>,
        }
        let resp = self
            .http
            .get(GITHUB_USERINFO_URL)
            .header("Authorization", format!("Bearer {github_oauth_token}"))
            .header("Accept", "application/json")
            .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
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
        let form_body = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("client_id", COPILOT_CLIENT_ID)
            .append_pair("scope", "read:user")
            .finish();
        let resp = self
            .http
            .post(GITHUB_DEVICE_CODE_URL)
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
            .post(GITHUB_ACCESS_TOKEN_URL)
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
            let (copilot_bearer, expires_at) = self
                .exchange_for_copilot_bearer(&github_oauth_token)
                .await?;
            let account_email = self.fetch_github_account_identity(&github_oauth_token).await;
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

    fn extra_headers_for(&self, _bundle: &OAuthTokenBundle) -> Vec<(String, String)> {
        vec![
            ("Editor-Version".to_owned(), COPILOT_EDITOR_VERSION.to_owned()),
            (
                "Editor-Plugin-Version".to_owned(),
                COPILOT_PLUGIN_VERSION.to_owned(),
            ),
            ("Copilot-Integration-Id".to_owned(), "vscode-chat".to_owned()),
            ("User-Agent".to_owned(), COPILOT_USER_AGENT.to_owned()),
            ("X-GitHub-Api-Version".to_owned(), GITHUB_API_VERSION.to_owned()),
        ]
    }
}
