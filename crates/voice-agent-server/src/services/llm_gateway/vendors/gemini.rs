use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use url::Url;

use super::{ExchangedOAuthAccount, OAuthVendorDriver, parse_reset_timestamp};
use crate::services::llm_gateway::types::{
    GatewayError, GatewayQuotaWindow, GatewayVendor, OAuthTokenBundle, VendorQuotaSnapshot,
};

const GEMINI_CLIENT_ID: &str =
    "681255809395-oo8ft2oprdrnp9e3aqf6av3hmdib135j.apps.googleusercontent.com";
const GEMINI_CLIENT_SECRET: &str = "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";
const GEMINI_AUTHORIZE_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GEMINI_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const GEMINI_USERINFO_URL: &str = "https://www.googleapis.com/oauth2/v1/userinfo?alt=json";
const GEMINI_LOAD_CODE_ASSIST_URL: &str =
    "https://cloudcode-pa.googleapis.com/v1internal:loadCodeAssist";
const GEMINI_RETRIEVE_QUOTA_URL: &str =
    "https://cloudcode-pa.googleapis.com/v1internal:retrieveUserQuota";
const GEMINI_REDIRECT_URI: &str = "http://localhost:8085/oauth2callback";
const GEMINI_SCOPES: &str = "https://www.googleapis.com/auth/cloud-platform https://www.googleapis.com/auth/userinfo.email https://www.googleapis.com/auth/userinfo.profile";

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

    async fn fetch_code_assist_context(
        &self,
        access_token: &str,
    ) -> (Option<String>, Option<String>) {
        let body = json!({
            "metadata": {
                "ideType": 9,
                "platform": 2,
                "pluginType": 2
            },
            "mode": 1
        });
        let Ok(resp) = self
            .http
            .post(GEMINI_LOAD_CODE_ASSIST_URL)
            .bearer_auth(access_token)
            .header("Accept", "application/json")
            .json(&body)
            .send()
            .await
        else {
            return (None, None);
        };
        if !resp.status().is_success() {
            return (None, None);
        }
        let Ok(data) = resp.json::<Value>().await else {
            return (None, None);
        };
        let project_id = data
            .get("cloudaicompanionProject")
            .and_then(|p| p.as_str().or_else(|| p.get("id").and_then(Value::as_str)))
            .filter(|s| !s.trim().is_empty())
            .map(str::to_owned);
        let plan = data
            .get("currentTier")
            .and_then(|t| t.get("name"))
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(str::to_owned);
        (project_id, plan)
    }
}

pub(crate) fn parse_gemini_quota_payload(
    data: &Value,
    plan: Option<String>,
    discovered_account_context_id: Option<String>,
) -> VendorQuotaSnapshot {
    let mut windows = Vec::new();
    let mut limit_reached = false;

    if let Some(buckets) = data.get("buckets").and_then(Value::as_array) {
        for bucket in buckets {
            let Some(model_id) = bucket
                .get("modelId")
                .and_then(Value::as_str)
                .filter(|s| !s.trim().is_empty())
            else {
                continue;
            };
            let remaining_fraction = bucket
                .get("remainingFraction")
                .and_then(Value::as_f64)
                .unwrap_or(1.0)
                .clamp(0.0, 1.0);
            let remaining_percentage = (remaining_fraction * 100.0).clamp(0.0, 100.0);
            let used = ((1.0 - remaining_fraction) * 1000.0).round().max(0.0);
            if remaining_percentage <= 0.0 {
                limit_reached = true;
            }
            let reset_at = parse_reset_timestamp(bucket.get("resetTime"));
            windows.push(GatewayQuotaWindow {
                name: model_id.to_owned(),
                used,
                total: 1000.0,
                remaining_percentage,
                reset_at,
                unlimited: false,
                unit: Some("req".to_owned()),
            });
        }
    }

    VendorQuotaSnapshot {
        plan: Some(plan.unwrap_or_else(|| "Free".to_owned())),
        limit_reached,
        windows,
        message: None,
        discovered_account_context_id,
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
            ("client_secret", GEMINI_CLIENT_SECRET),
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
            .header("Accept", "application/json")
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
        let (project_id, _) = self.fetch_code_assist_context(&parsed.access_token).await;
        let now = crate::database::unix_seconds().unwrap_or(0);
        let expires_at = parsed.expires_in.map(|ttl| now + ttl.max(60));
        Ok(ExchangedOAuthAccount {
            bundle: OAuthTokenBundle {
                access_token: parsed.access_token,
                refresh_token: parsed.refresh_token,
                upstream_oauth_token: None,
                account_context_id: project_id,
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
            ("client_secret", GEMINI_CLIENT_SECRET),
            ("refresh_token", refresh_token),
        ];
        let encoded = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(params)
            .finish();
        let resp = self
            .http
            .post(GEMINI_TOKEN_URL)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("Accept", "application/json")
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

    async fn fetch_quota(
        &self,
        bundle: &OAuthTokenBundle,
    ) -> Result<VendorQuotaSnapshot, GatewayError> {
        let existing_project = bundle
            .account_context_id
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .map(str::to_owned);
        let (discovered_project, plan) = self.fetch_code_assist_context(&bundle.access_token).await;
        let project_id = existing_project
            .clone()
            .or_else(|| discovered_project.clone());
        let Some(project_id) = project_id else {
            return Ok(VendorQuotaSnapshot {
                plan: Some(plan.unwrap_or_else(|| "Free".to_owned())),
                limit_reached: false,
                windows: Vec::new(),
                message: Some(
                    "Unable to discover Gemini Cloud Code project ID. Try re-authenticating."
                        .to_owned(),
                ),
                discovered_account_context_id: None,
            });
        };

        let resp = self
            .http
            .post(GEMINI_RETRIEVE_QUOTA_URL)
            .bearer_auth(&bundle.access_token)
            .header("Accept", "application/json")
            .json(&json!({ "project": project_id }))
            .send()
            .await
            .map_err(|e| GatewayError::OAuthExchangeFailed(format!("gemini_quota_error: {e}")))?;
        if resp.status().as_u16() == 401 {
            return Err(GatewayError::AccountReauthRequired);
        }
        if !resp.status().is_success() {
            return Err(GatewayError::OAuthExchangeFailed(format!(
                "gemini_quota_http_{}",
                resp.status().as_u16()
            )));
        }
        let payload: Value = resp
            .json()
            .await
            .map_err(|e| GatewayError::OAuthExchangeFailed(format!("gemini_quota_json: {e}")))?;
        let discovered_to_persist = if existing_project.is_none() {
            Some(project_id)
        } else {
            None
        };
        Ok(parse_gemini_quota_payload(
            &payload,
            plan,
            discovered_to_persist,
        ))
    }

    fn extra_headers_for(&self, _bundle: &OAuthTokenBundle) -> Vec<(String, String)> {
        Vec::new()
    }
}
