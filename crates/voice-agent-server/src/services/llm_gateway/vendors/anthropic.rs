use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use url::Url;

use super::{ExchangedOAuthAccount, OAuthVendorDriver};
use crate::services::llm_gateway::types::{GatewayError, GatewayVendor, OAuthTokenBundle};

const ANTHROPIC_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const ANTHROPIC_AUTHORIZE_URL: &str = "https://claude.ai/oauth/authorize";
const ANTHROPIC_TOKEN_URL: &str = "https://console.anthropic.com/v1/oauth/token";
const ANTHROPIC_REDIRECT_URI: &str = "https://console.anthropic.com/oauth/code/callback";
const ANTHROPIC_SCOPES: &str = "org:create_api_key user:profile user:inference";

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
        let body = json!({
            "grant_type": "authorization_code",
            "client_id": ANTHROPIC_CLIENT_ID,
            "code": code,
            "state": state,
            "redirect_uri": redirect_uri,
            "code_verifier": code_verifier,
        });
        let resp = self
            .http
            .post(ANTHROPIC_TOKEN_URL)
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

    fn extra_headers_for(&self, _bundle: &OAuthTokenBundle) -> Vec<(String, String)> {
        vec![
            ("anthropic-version".to_owned(), "2023-06-01".to_owned()),
            ("anthropic-beta".to_owned(), "oauth-2025-04-20".to_owned()),
        ]
    }
}
