use std::time::Duration;

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::Deserialize;
use serde_json::{Value, json};
use url::Url;

use super::{ExchangedOAuthAccount, OAuthVendorDriver};
use crate::services::llm_gateway::types::{GatewayError, GatewayVendor, OAuthTokenBundle};

const CODEX_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const CODEX_AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
const CODEX_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const CODEX_REDIRECT_URI: &str = "http://localhost:1455/auth/callback";
const CODEX_SCOPES: &str = "openid profile email offline_access";

pub struct OpenAiCodexOAuthDriver {
    http: reqwest::Client,
}

impl OpenAiCodexOAuthDriver {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .unwrap_or_default();
        Self { http }
    }
}

#[derive(Deserialize)]
struct OpenAiTokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    id_token: Option<String>,
    expires_in: Option<i64>,
}

fn extract_jwt_claims(token: &str) -> (Option<String>, Option<String>) {
    let mut parts = token.split('.');
    let _header = parts.next();
    let Some(payload_b64) = parts.next() else {
        return (None, None);
    };
    let Ok(bytes) = URL_SAFE_NO_PAD.decode(payload_b64.as_bytes()) else {
        return (None, None);
    };
    let Ok(val) = serde_json::from_slice::<Value>(&bytes) else {
        return (None, None);
    };
    let email = val
        .get("email")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let account_id = val
        .get("https://api.openai.com/auth")
        .and_then(|v| v.get("chatgpt_account_id"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    (email, account_id)
}

#[async_trait]
impl OAuthVendorDriver for OpenAiCodexOAuthDriver {
    fn vendor(&self) -> GatewayVendor {
        GatewayVendor::OpenaiCodex
    }

    fn default_redirect_uri(&self) -> &'static str {
        CODEX_REDIRECT_URI
    }

    fn build_pkce_authorize_url(
        &self,
        state: &str,
        code_challenge: &str,
        redirect_uri: &str,
    ) -> Result<String, GatewayError> {
        let mut url =
            Url::parse(CODEX_AUTHORIZE_URL).map_err(|_| GatewayError::OAuthCallbackInvalid)?;
        url.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", CODEX_CLIENT_ID)
            .append_pair("redirect_uri", redirect_uri)
            .append_pair("scope", CODEX_SCOPES)
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
        let body = json!({
            "grant_type": "authorization_code",
            "client_id": CODEX_CLIENT_ID,
            "code": code,
            "redirect_uri": redirect_uri,
            "code_verifier": code_verifier,
        });
        let resp = self
            .http
            .post(CODEX_TOKEN_URL)
            .json(&body)
            .send()
            .await
            .map_err(|e| GatewayError::OAuthExchangeFailed(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(GatewayError::OAuthExchangeFailed(format!(
                "codex_token_http_{}",
                resp.status().as_u16()
            )));
        }
        let parsed: OpenAiTokenResponse = resp
            .json()
            .await
            .map_err(|e| GatewayError::OAuthExchangeFailed(e.to_string()))?;
        let (email_from_id, acct_from_id) = parsed
            .id_token
            .as_deref()
            .map(extract_jwt_claims)
            .unwrap_or((None, None));
        let (email_from_acc, acct_from_acc) = extract_jwt_claims(&parsed.access_token);
        let now = crate::database::unix_seconds().unwrap_or(0);
        let expires_at = parsed.expires_in.map(|ttl| now + ttl.max(60));
        Ok(ExchangedOAuthAccount {
            bundle: OAuthTokenBundle {
                access_token: parsed.access_token,
                refresh_token: parsed.refresh_token,
                upstream_oauth_token: None,
                account_context_id: acct_from_acc.or(acct_from_id),
            },
            account_email: email_from_id.or(email_from_acc),
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
            "client_id": CODEX_CLIENT_ID,
            "refresh_token": refresh_token,
        });
        let resp = self
            .http
            .post(CODEX_TOKEN_URL)
            .json(&body)
            .send()
            .await
            .map_err(|e| GatewayError::OAuthExchangeFailed(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(GatewayError::AccountReauthRequired);
        }
        let parsed: OpenAiTokenResponse = resp
            .json()
            .await
            .map_err(|e| GatewayError::OAuthExchangeFailed(e.to_string()))?;
        let (email, acct) = extract_jwt_claims(&parsed.access_token);
        let now = crate::database::unix_seconds().unwrap_or(0);
        let expires_at = parsed.expires_in.map(|ttl| now + ttl.max(60));
        Ok(ExchangedOAuthAccount {
            bundle: OAuthTokenBundle {
                access_token: parsed.access_token,
                refresh_token: parsed
                    .refresh_token
                    .or_else(|| current.refresh_token.clone()),
                upstream_oauth_token: None,
                account_context_id: acct.or_else(|| current.account_context_id.clone()),
            },
            account_email: email,
            expires_at,
        })
    }

    fn extra_headers_for(&self, bundle: &OAuthTokenBundle) -> Vec<(String, String)> {
        bundle
            .account_context_id
            .as_ref()
            .map(|id| vec![("ChatGPT-Account-Id".to_owned(), id.clone())])
            .unwrap_or_default()
    }
}
