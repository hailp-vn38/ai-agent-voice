use std::time::Duration;

use async_trait::async_trait;
use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, URL_SAFE, URL_SAFE_NO_PAD},
};
use serde::Deserialize;
use serde_json::{Value, json};
use url::Url;

use super::{ExchangedOAuthAccount, OAuthVendorDriver};
use crate::services::llm_gateway::types::{GatewayError, GatewayVendor, OAuthTokenBundle};

const CODEX_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const CODEX_AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
const CODEX_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
pub const CODEX_REDIRECT_URI: &str = "http://localhost:1455/auth/callback";
pub const CODEX_FIXED_PORT: u16 = 1455;
const CODEX_SCOPES: &str = "openid profile email offline_access";
const CODEX_ORIGINATOR: &str = "codex_cli_rs";
const CODEX_VERSION: &str = "0.159.0";

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

fn decode_jwt_payload(token: &str) -> Option<Value> {
    let mut parts = token.split('.');
    let _header = parts.next()?;
    let payload_b64 = parts.next()?;
    let bytes = URL_SAFE_NO_PAD
        .decode(payload_b64.as_bytes())
        .or_else(|_| URL_SAFE.decode(payload_b64.as_bytes()))
        .or_else(|_| {
            let normalized = payload_b64.replace('-', "+").replace('_', "/");
            let pad = (4 - normalized.len() % 4) % 4;
            let padded = format!("{normalized}{}", "=".repeat(pad));
            STANDARD.decode(padded.as_bytes())
        })
        .ok()?;
    serde_json::from_slice::<Value>(&bytes).ok()
}

pub(crate) fn extract_jwt_claims(token: &str) -> (Option<String>, Option<String>) {
    let Some(val) = decode_jwt_payload(token) else {
        return (None, None);
    };
    let profile_email = val
        .get("https://api.openai.com/profile")
        .and_then(|v| v.get("email"))
        .and_then(Value::as_str);
    let email = val
        .get("email")
        .and_then(Value::as_str)
        .or(profile_email)
        .or_else(|| val.get("preferred_username").and_then(Value::as_str))
        .or_else(|| val.get("upn").and_then(Value::as_str))
        .filter(|s| !s.trim().is_empty())
        .map(str::to_owned);

    let auth_obj = val.get("https://api.openai.com/auth");
    let account_id = auth_obj
        .and_then(|v| v.get("chatgpt_account_id"))
        .and_then(Value::as_str)
        .or_else(|| val.get("chatgpt_account_id").and_then(Value::as_str))
        .or_else(|| val.get("account_id").and_then(Value::as_str))
        .filter(|s| !s.trim().is_empty())
        .map(str::to_owned);
    (email, account_id)
}

fn extract_jwt_exp(token: &str) -> Option<i64> {
    decode_jwt_payload(token)?
        .get("exp")
        .and_then(Value::as_i64)
}

fn encode_uri_component(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'!'
            | b'~'
            | b'*'
            | b'\''
            | b'('
            | b')' => out.push(byte as char),
            _ => {
                use std::fmt::Write as _;
                let _ = write!(&mut out, "%{byte:02X}");
            }
        }
    }
    out
}

#[async_trait]
impl OAuthVendorDriver for OpenAiCodexOAuthDriver {
    fn vendor(&self) -> GatewayVendor {
        GatewayVendor::OpenaiCodex
    }

    fn default_redirect_uri(&self) -> &'static str {
        CODEX_REDIRECT_URI
    }

    fn requires_fixed_redirect_uri(&self) -> bool {
        true
    }

    fn build_pkce_authorize_url(
        &self,
        state: &str,
        code_challenge: &str,
        redirect_uri: &str,
    ) -> Result<String, GatewayError> {
        let _ = Url::parse(CODEX_AUTHORIZE_URL).map_err(|_| GatewayError::OAuthCallbackInvalid)?;
        let effective_redirect = if redirect_uri.trim().is_empty() {
            CODEX_REDIRECT_URI
        } else {
            redirect_uri.trim()
        };
        // Match 9router (`src/lib/oauth/providers/codex.js` & `open-sse/providers/registry/codex.js`):
        // include `id_token_add_organizations=true`, `codex_cli_simplified_flow=true`,
        // `originator=codex_cli_rs`, and percent-encode with `%20` (not `+`).
        let params = [
            ("response_type", "code"),
            ("client_id", CODEX_CLIENT_ID),
            ("redirect_uri", effective_redirect),
            ("scope", CODEX_SCOPES),
            ("code_challenge", code_challenge),
            ("code_challenge_method", "S256"),
            ("id_token_add_organizations", "true"),
            ("codex_cli_simplified_flow", "true"),
            ("originator", CODEX_ORIGINATOR),
            ("state", state),
        ];
        let query = params
            .iter()
            .map(|(k, v)| format!("{k}={}", encode_uri_component(v)))
            .collect::<Vec<_>>()
            .join("&");
        Ok(format!("{CODEX_AUTHORIZE_URL}?{query}"))
    }

    async fn exchange_pkce_code(
        &self,
        code: &str,
        _state: &str,
        code_verifier: &str,
        redirect_uri: &str,
    ) -> Result<ExchangedOAuthAccount, GatewayError> {
        let trimmed_code = code.trim();
        // Support direct JWT access token paste (matching 9router `route.js` `eyJ...` handling)
        if trimmed_code.starts_with("eyJ") && trimmed_code.contains('.') {
            let (email, account_id) = extract_jwt_claims(trimmed_code);
            let expires_at = extract_jwt_exp(trimmed_code);
            return Ok(ExchangedOAuthAccount {
                bundle: OAuthTokenBundle {
                    access_token: trimmed_code.to_owned(),
                    refresh_token: None,
                    upstream_oauth_token: None,
                    account_context_id: account_id,
                },
                account_email: email,
                expires_at,
            });
        }

        let effective_redirect = if redirect_uri.trim().is_empty() {
            CODEX_REDIRECT_URI
        } else {
            redirect_uri.trim()
        };
        let form_body = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("grant_type", "authorization_code")
            .append_pair("client_id", CODEX_CLIENT_ID)
            .append_pair("code", trimmed_code)
            .append_pair("redirect_uri", effective_redirect)
            .append_pair("code_verifier", code_verifier)
            .finish();

        let resp = self
            .http
            .post(CODEX_TOKEN_URL)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("Accept", "application/json")
            .body(form_body)
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
        let expires_at = parsed
            .expires_in
            .map(|ttl| now + ttl.max(60))
            .or_else(|| extract_jwt_exp(&parsed.access_token));
        Ok(ExchangedOAuthAccount {
            bundle: OAuthTokenBundle {
                access_token: parsed.access_token,
                refresh_token: parsed.refresh_token,
                upstream_oauth_token: None,
                account_context_id: acct_from_id.or(acct_from_acc),
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
        let mut resp = self
            .http
            .post(CODEX_TOKEN_URL)
            .header("Accept", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| GatewayError::OAuthExchangeFailed(e.to_string()))?;
        if !resp.status().is_success() {
            let form_body = url::form_urlencoded::Serializer::new(String::new())
                .append_pair("grant_type", "refresh_token")
                .append_pair("client_id", CODEX_CLIENT_ID)
                .append_pair("refresh_token", refresh_token)
                .finish();
            resp = self
                .http
                .post(CODEX_TOKEN_URL)
                .header("Content-Type", "application/x-www-form-urlencoded")
                .header("Accept", "application/json")
                .body(form_body)
                .send()
                .await
                .map_err(|e| GatewayError::OAuthExchangeFailed(e.to_string()))?;
        }
        if !resp.status().is_success() {
            return Err(GatewayError::AccountReauthRequired);
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
        let expires_at = parsed
            .expires_in
            .map(|ttl| now + ttl.max(60))
            .or_else(|| extract_jwt_exp(&parsed.access_token));
        Ok(ExchangedOAuthAccount {
            bundle: OAuthTokenBundle {
                access_token: parsed.access_token,
                refresh_token: parsed
                    .refresh_token
                    .or_else(|| current.refresh_token.clone()),
                upstream_oauth_token: None,
                account_context_id: acct_from_id
                    .or(acct_from_acc)
                    .or_else(|| current.account_context_id.clone()),
            },
            account_email: email_from_id.or(email_from_acc),
            expires_at,
        })
    }

    fn extra_headers_for(&self, bundle: &OAuthTokenBundle) -> Vec<(String, String)> {
        let mut headers = vec![
            ("originator".to_owned(), CODEX_ORIGINATOR.to_owned()),
            ("Version".to_owned(), CODEX_VERSION.to_owned()),
        ];
        if let Some(id) = bundle.account_context_id.as_ref().filter(|s| !s.is_empty()) {
            headers.push(("ChatGPT-Account-Id".to_owned(), id.clone()));
        }
        headers
    }
}
