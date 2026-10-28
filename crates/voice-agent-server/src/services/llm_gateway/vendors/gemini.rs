use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use url::Url;

use super::{ExchangedOAuthAccount, OAuthVendorDriver};
use crate::services::llm_gateway::types::{GatewayError, GatewayVendor, OAuthTokenBundle};

const GEMINI_CLIENT_ID: &str =
    "681255809395-oo8ft2oprdrnp9e3aqf6av3hmdib135j.apps.googleusercontent.com";
const GEMINI_AUTHORIZE_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GEMINI_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const GEMINI_USERINFO_URL: &str = "https://www.googleapis.com/oauth2/v2/userinfo";
const GEMINI_REDIRECT_URI: &str = "http://localhost:8085/oauth2callback";
const GEMINI_SCOPES: &str =
    "https://www.googleapis.com/auth/cloud-platform https://www.googleapis.com/auth/userinfo.email openid";

pub struct GeminiOAuthDriver {
    http: reqwest::Client,
}

impl GeminiOAuthDriver {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .unwrap_or_default();
        Self { http }
    }

    async fn fetch_email(&self, access_token: &str) -> Option<String> {
        #[derive(Deserialize)]
        struct UserInfo {
            email: Option<String>,
        }
        let resp = self
            .http
            .get(GEMINI_USERINFO_URL)
            .bearer_auth(access_token)
            .send()
            .await
            .ok()?;
        let info: UserInfo = resp.json().await.ok()?;
        info.email
    }
}

#[derive(Deserialize)]
struct GeminiTokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
}

#[async_trait]
impl OAuthVendorDriver for GeminiOAuthDriver {
    fn vendor(&self) -> GatewayVendor {
        GatewayVendor::Gemini
    }

    fn default_redirect_uri(&self) -> &'static str {
        GEMINI_REDIRECT_URI
    }

    fn build_pkce_authorize_url(
        &self,
        state: &str,
        code_challenge: &str,
        redirect_uri: &str,
    ) -> Result<String, GatewayError> {
        let mut url =
            Url::parse(GEMINI_AUTHORIZE_URL).map_err(|_| GatewayError::OAuthCallbackInvalid)?;
        url.query_pairs_mut()
            .append_pair("client_id", GEMINI_CLIENT_ID)
            .append_pair("response_type", "code")
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("scope", GEMINI_SCOPES)
            .append_pair("access_type", "offline")
            .append_pair("prompt", "consent")
            .append_pair("code_challenge", code_challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", state);
        Ok(url.to_string())
    }

    async fn exchange_pkce_code(
        &self,
        code: &str,
        _state: &str,
        code_verifier: &str,
        redirect_uri: &str,
    ) -> Result<ExchangedOAuthAccount, GatewayError> {
        let params = [
            ("grant_type", "authorization_code"),
            ("client_id", GEMINI_CLIENT_ID),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("code_verifier", code_verifier),
        ];
        let encoded = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(params)
            .finish();
        let resp = self
            .http
            .post(GEMINI_TOKEN_URL)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(encoded)
            .send()
            .await
            .map_err(|e| GatewayError::OAuthExchangeFailed(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(GatewayError::OAuthExchangeFailed(format!(
                "gemini_token_http_{}",
                resp.status().as_u16()
            )));
        }
        let parsed: GeminiTokenResponse = resp
            .json()
            .await
            .map_err(|e| GatewayError::OAuthExchangeFailed(e.to_string()))?;
        let email = self.fetch_email(&parsed.access_token).await;
        let now = crate::database::unix_seconds().unwrap_or(0);
        let expires_at = parsed.expires_in.map(|ttl| now + ttl.max(60));
        Ok(ExchangedOAuthAccount {
            bundle: OAuthTokenBundle {
                access_token: parsed.access_token,
                refresh_token: parsed.refresh_token,
                upstream_oauth_token: None,
                account_context_id: None,
            },
            account_email: email,
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
        let params = [
            ("grant_type", "refresh_token"),
            ("client_id", GEMINI_CLIENT_ID),
            ("refresh_token", refresh_token),
        ];
        let encoded = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(params)
            .finish();
        let resp = self
            .http
            .post(GEMINI_TOKEN_URL)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(encoded)
            .send()
            .await
            .map_err(|e| GatewayError::OAuthExchangeFailed(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(GatewayError::AccountReauthRequired);
        }
        let parsed: GeminiTokenResponse = resp
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
            account_email: None,
            expires_at,
        })
    }

    fn extra_headers_for(&self, _bundle: &OAuthTokenBundle) -> Vec<(String, String)> {
        Vec::new()
    }
}
