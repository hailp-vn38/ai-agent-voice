use std::time::Duration;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use url::Url;

use super::{ExchangedOAuthAccount, OAuthVendorDriver, parse_reset_timestamp};
use crate::services::llm_gateway::types::{
    GatewayError, GatewayQuotaWindow, GatewayVendor, OAuthTokenBundle, VendorQuotaSnapshot,
};

const ANTIGRAVITY_CLIENT_ID: &str =
    "1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com";
const ANTIGRAVITY_CLIENT_SECRET: &str = "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";
const GEMINI_CLI_CLIENT_ID: &str =
    "681255809395-oo8ft2oprdrnp9e3aqf6av3hmdib135j.apps.googleusercontent.com";
const GEMINI_CLI_CLIENT_SECRET: &str = "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";
const GEMINI_CLIENT_CANDIDATES: &[(&str, &str)] = &[
    (ANTIGRAVITY_CLIENT_ID, ANTIGRAVITY_CLIENT_SECRET),
    (GEMINI_CLI_CLIENT_ID, GEMINI_CLI_CLIENT_SECRET),
];

pub(crate) const ANTIGRAVITY_IDE_VERSION: &str = "2.24.0";
pub(crate) const ANTIGRAVITY_USER_AGENT: &str = "antigravity/hub/2.24.0 darwin/arm64";

const GEMINI_AUTHORIZE_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GEMINI_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const GEMINI_USERINFO_URL: &str = "https://www.googleapis.com/oauth2/v1/userinfo?alt=json";
const GEMINI_LOAD_CODE_ASSIST_URL: &str =
    "https://cloudcode-pa.googleapis.com/v1internal:loadCodeAssist";
const GEMINI_ONBOARD_USER_URL: &str = "https://cloudcode-pa.googleapis.com/v1internal:onboardUser";
const GEMINI_FETCH_MODELS_QUOTA_URL: &str =
    "https://daily-cloudcode-pa.googleapis.com/v1internal:fetchAvailableModels";
const GEMINI_QUOTA_SUMMARY_URL: &str =
    "https://daily-cloudcode-pa.googleapis.com/v1internal:retrieveUserQuotaSummary";
const GEMINI_REDIRECT_URI: &str = "http://localhost:8085/oauth2callback";
const GEMINI_SCOPES: &str = "https://www.googleapis.com/auth/cloud-platform https://www.googleapis.com/auth/userinfo.email https://www.googleapis.com/auth/userinfo.profile https://www.googleapis.com/auth/cclog https://www.googleapis.com/auth/experimentsandconfigs";

const IMPORTANT_ANTIGRAVITY_MODELS: &[&str] = &[
    "gemini-4-argon-high",
    "gemini-4-argon-medium",
    "gemini-4-argon-low",
    "gemini-3.8-flash-high",
    "gemini-3.8-flash-medium",
    "gemini-3.8-flash-low",
    "gemini-3.7-flash-high",
    "gemini-3.7-flash-medium",
    "gemini-3.7-flash-low",
    "gemini-3.6-flash-high",
    "gemini-3.6-flash-medium",
    "gemini-3.6-flash-low",
    "gemini-3.1-pro-high",
    "gemini-pro-agent",
    "gemini-3.1-pro-low",
    "claude-sonnet-5-5-high",
    "claude-sonnet-5-5-medium",
    "claude-sonnet-5-5-low",
    "claude-sonnet-5-5",
    "claude-opus-5-5-high",
    "claude-opus-5-5-medium",
    "claude-opus-5-5-low",
    "claude-opus-5-5",
    "claude-sonnet-4-6",
    "claude-opus-4-6-thinking",
    "gpt-oss-120b-medium",
    "gemini-3.1-flash-image",
];

struct CodeAssistContext {
    project_id: Option<String>,
    plan: Option<String>,
    is_free_tier: bool,
    error_message: Option<String>,
}

pub struct GeminiOAuthDriver {
    http: reqwest::Client,
    token_url: String,
    userinfo_url: String,
    load_code_assist_url: String,
    onboard_user_url: String,
    retrieve_quota_url: String,
    quota_summary_url: String,
}

impl Default for GeminiOAuthDriver {
    fn default() -> Self {
        Self::new()
    }
}

impl GeminiOAuthDriver {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .unwrap_or_default();
        Self {
            http,
            token_url: GEMINI_TOKEN_URL.to_owned(),
            userinfo_url: GEMINI_USERINFO_URL.to_owned(),
            load_code_assist_url: GEMINI_LOAD_CODE_ASSIST_URL.to_owned(),
            onboard_user_url: GEMINI_ONBOARD_USER_URL.to_owned(),
            retrieve_quota_url: GEMINI_FETCH_MODELS_QUOTA_URL.to_owned(),
            quota_summary_url: GEMINI_QUOTA_SUMMARY_URL.to_owned(),
        }
    }

    #[cfg(test)]
    pub fn with_test_endpoints(base_url: &str) -> Self {
        let base = base_url.trim_end_matches('/');
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap_or_default();
        Self {
            http,
            token_url: format!("{base}/token"),
            userinfo_url: format!("{base}/userinfo"),
            load_code_assist_url: format!("{base}/v1internal:loadCodeAssist"),
            onboard_user_url: format!("{base}/v1internal:onboardUser"),
            retrieve_quota_url: format!("{base}/v1internal:fetchAvailableModels"),
            quota_summary_url: format!("{base}/v1internal:retrieveUserQuotaSummary"),
        }
    }

    async fn fetch_email(&self, access_token: &str) -> Option<String> {
        #[derive(Deserialize)]
        struct UserInfo {
            email: Option<String>,
        }
        let resp = self
            .http
            .get(&self.userinfo_url)
            .bearer_auth(access_token)
            .send()
            .await
            .ok()?;
        let info: UserInfo = resp.json().await.ok()?;
        info.email
    }

    async fn post_token_with_candidates(
        &self,
        grant_type: &str,
        extra_params: &[(&str, &str)],
    ) -> Result<(Option<GeminiTokenResponse>, u16), GatewayError> {
        let mut last_status = 0u16;
        for &(client_id, client_secret) in GEMINI_CLIENT_CANDIDATES {
            let encoded = {
                let mut serializer = url::form_urlencoded::Serializer::new(String::new());
                serializer
                    .append_pair("grant_type", grant_type)
                    .append_pair("client_id", client_id)
                    .append_pair("client_secret", client_secret);
                serializer.extend_pairs(extra_params.iter().copied());
                serializer.finish()
            };
            let resp = self
                .http
                .post(&self.token_url)
                .header("Content-Type", "application/x-www-form-urlencoded")
                .header("Accept", "application/json")
                .body(encoded)
                .send()
                .await
                .map_err(|e| GatewayError::OAuthExchangeFailed(e.to_string()))?;
            last_status = resp.status().as_u16();
            if resp.status().is_success() {
                let parsed: GeminiTokenResponse = resp
                    .json()
                    .await
                    .map_err(|e| GatewayError::OAuthExchangeFailed(e.to_string()))?;
                return Ok((Some(parsed), last_status));
            }
        }
        Ok((None, last_status))
    }

    fn extract_project_id(val: &Value) -> Option<String> {
        val.get("cloudaicompanionProject")
            .and_then(|p| p.as_str().or_else(|| p.get("id").and_then(Value::as_str)))
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    }

    async fn onboard_user(&self, access_token: &str, tier_id: &str) -> Option<String> {
        let body = json!({
            "tierId": tier_id,
            "metadata": {
                "ideType": 9,
                "platform": 2,
                "pluginType": 2
            }
        });
        for attempt in 0..3 {
            let Ok(resp) = self
                .http
                .post(&self.onboard_user_url)
                .bearer_auth(access_token)
                .header("Accept", "application/json")
                .header("User-Agent", ANTIGRAVITY_USER_AGENT)
                .json(&body)
                .send()
                .await
            else {
                return None;
            };
            if !resp.status().is_success() {
                return None;
            }
            let Ok(data) = resp.json::<Value>().await else {
                return None;
            };
            if data.get("done").and_then(Value::as_bool) == Some(true) {
                if let Some(resp_obj) = data.get("response")
                    && let Some(pid) = Self::extract_project_id(resp_obj)
                {
                    return Some(pid);
                }
                return Self::extract_project_id(&data);
            }
            if attempt < 2 {
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
        None
    }

    fn extract_error_message(text: &str) -> Option<String> {
        let parsed: Value = serde_json::from_str(text).ok()?;
        parsed
            .get("error")
            .and_then(|e| e.get("message").or(Some(e)))
            .and_then(Value::as_str)
            .or_else(|| parsed.get("message").and_then(Value::as_str))
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    }

    async fn fetch_code_assist_context(
        &self,
        access_token: &str,
        run_onboard_if_needed: bool,
    ) -> CodeAssistContext {
        // NOTE: Do NOT send `X-Goog-Api-Client` or `Client-Metadata` headers here;
        // Google blocks `cloudaicompanionProject` provisioning when those are present on Antigravity.
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
            .post(&self.load_code_assist_url)
            .bearer_auth(access_token)
            .header("Accept", "application/json")
            .header("User-Agent", ANTIGRAVITY_USER_AGENT)
            .json(&body)
            .send()
            .await
        else {
            return CodeAssistContext {
                project_id: None,
                plan: None,
                is_free_tier: true,
                error_message: None,
            };
        };
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let text = resp.text().await.unwrap_or_default();
            let err_msg = Self::extract_error_message(&text)
                .unwrap_or_else(|| format!("Cloud Code Assist returned HTTP {status}"));
            return CodeAssistContext {
                project_id: None,
                plan: None,
                is_free_tier: true,
                error_message: Some(err_msg),
            };
        }
        let Ok(data) = resp.json::<Value>().await else {
            return CodeAssistContext {
                project_id: None,
                plan: None,
                is_free_tier: true,
                error_message: None,
            };
        };
        let mut project_id = Self::extract_project_id(&data);
        let paid_tier_id = data
            .get("paidTier")
            .and_then(|t| t.get("id"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let is_free_tier = paid_tier_id.is_none_or(|id| id == "free-tier");
        let paid_tier_name = data
            .get("paidTier")
            .and_then(|t| t.get("name"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        let current_tier_name = data
            .get("currentTier")
            .and_then(|t| t.get("name").or_else(|| t.get("id")))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        let plan = if !is_free_tier {
            paid_tier_name.or(current_tier_name)
        } else {
            current_tier_name.or(paid_tier_name)
        };

        let tier_id = data
            .get("allowedTiers")
            .and_then(Value::as_array)
            .and_then(|tiers| {
                tiers.iter().find_map(|tier| {
                    (tier.get("isDefault").and_then(Value::as_bool) == Some(true))
                        .then(|| tier.get("id").and_then(Value::as_str))
                        .flatten()
                })
            })
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("legacy-tier")
            .to_owned();

        if run_onboard_if_needed {
            if project_id.is_none() {
                project_id = self.onboard_user(access_token, &tier_id).await;
            } else {
                // Fire-and-forget onboarding when project_id is already present (matches 9router postExchange)
                let http = self.http.clone();
                let url = self.onboard_user_url.clone();
                let tok = access_token.to_owned();
                tokio::spawn(async move {
                    let onboard_body = json!({
                        "tierId": tier_id,
                        "metadata": {
                            "ideType": 9,
                            "platform": 2,
                            "pluginType": 2
                        }
                    });
                    let _ = http
                        .post(&url)
                        .bearer_auth(&tok)
                        .header("Accept", "application/json")
                        .header("User-Agent", ANTIGRAVITY_USER_AGENT)
                        .json(&onboard_body)
                        .send()
                        .await;
                });
            }
        }
        CodeAssistContext {
            project_id,
            plan,
            is_free_tier,
            error_message: None,
        }
    }

    async fn fetch_weekly_summary_windows(
        &self,
        access_token: &str,
        project_id: Option<&str>,
    ) -> Vec<GatewayQuotaWindow> {
        let body = match project_id {
            Some(pid) if !pid.trim().is_empty() => json!({ "project": pid }),
            _ => json!({}),
        };
        let Ok(resp) = self
            .http
            .post(&self.quota_summary_url)
            .bearer_auth(access_token)
            .header("Accept", "application/json")
            .header("User-Agent", ANTIGRAVITY_USER_AGENT)
            .header("X-Client-Name", "antigravity")
            .header("X-Client-Version", ANTIGRAVITY_IDE_VERSION)
            .json(&body)
            .send()
            .await
        else {
            return Vec::new();
        };
        if !resp.status().is_success() {
            return Vec::new();
        }
        let Ok(data) = resp.json::<Value>().await else {
            return Vec::new();
        };
        parse_weekly_quota_summary(&data)
    }
}

pub(crate) fn parse_weekly_quota_summary(data: &Value) -> Vec<GatewayQuotaWindow> {
    let groups = data
        .get("groups")
        .and_then(Value::as_array)
        .or_else(|| {
            data.get("quotaSummary")
                .and_then(|q| q.get("groups"))
                .and_then(Value::as_array)
        });
    let Some(groups) = groups else {
        return Vec::new();
    };

    let mut windows = Vec::new();
    let mut seen_names = std::collections::HashSet::new();

    for group in groups {
        let display_name = group
            .get("displayName")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_ascii_lowercase();
        let is_gemini = display_name.contains("gemini");
        let is_claude_gpt = display_name.contains("claude") || display_name.contains("gpt");
        if !is_gemini && !is_claude_gpt {
            continue;
        }
        let Some(buckets) = group.get("buckets").and_then(Value::as_array) else {
            continue;
        };
        for bucket in buckets {
            let window_type = bucket
                .get("window")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_ascii_lowercase();
            let bucket_text = format!(
                "{} {}",
                bucket
                    .get("bucketId")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
                bucket
                    .get("displayName")
                    .and_then(Value::as_str)
                    .unwrap_or("")
            )
            .to_ascii_lowercase();
            let is_weekly = window_type == "weekly" || bucket_text.contains("weekly");
            let is_session = window_type == "5h"
                || window_type == "daily"
                || bucket_text.contains("five hour")
                || bucket_text.contains("5h")
                || bucket_text.contains("daily");
            if !is_weekly && !is_session {
                continue;
            }
            let disabled = bucket
                .get("disabled")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if disabled && is_weekly {
                continue;
            }
            let remaining_fraction = if disabled {
                0.0
            } else {
                let Some(f) = bucket.get("remainingFraction").and_then(Value::as_f64) else {
                    continue;
                };
                f.clamp(0.0, 1.0)
            };
            let label = match (is_gemini, is_weekly) {
                (true, true) => "Gemini (Weekly)",
                (true, false) => "Gemini (5h)",
                (false, true) => "Claude & GPT (Weekly)",
                (false, false) => "Claude & GPT (5h)",
            };
            if !seen_names.insert(label.to_owned()) {
                continue;
            }
            let remaining_percentage = (remaining_fraction * 100.0).clamp(0.0, 100.0);
            let used = ((1.0 - remaining_fraction) * 1000.0).round().max(0.0);
            let reset_at = parse_reset_timestamp(bucket.get("resetTime"));
            windows.push(GatewayQuotaWindow {
                name: label.to_owned(),
                used,
                total: 1000.0,
                remaining_percentage,
                reset_at,
                unlimited: false,
                unit: Some("req".to_owned()),
            });
        }
    }
    windows
}

pub(crate) fn parse_gemini_quota_payload(
    data: &Value,
    plan: Option<String>,
    discovered_account_context_id: Option<String>,
) -> VendorQuotaSnapshot {
    parse_gemini_quota_payload_with_tier(data, plan, discovered_account_context_id, false)
}

fn parse_gemini_quota_payload_with_tier(
    data: &Value,
    plan: Option<String>,
    discovered_account_context_id: Option<String>,
    is_free_tier: bool,
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
    } else if !is_free_tier
        && let Some(models_obj) = data.get("models").and_then(Value::as_object)
    {
        for &model_key in IMPORTANT_ANTIGRAVITY_MODELS {
            let Some(info) = models_obj.get(model_key) else {
                continue;
            };
            if info.get("isInternal").and_then(Value::as_bool) == Some(true) {
                continue;
            }
            let Some(quota_info) = info.get("quotaInfo") else {
                continue;
            };
            let remaining_fraction = quota_info
                .get("remainingFraction")
                .and_then(Value::as_f64)
                .unwrap_or(0.0)
                .clamp(0.0, 1.0);
            let remaining_percentage = (remaining_fraction * 100.0).clamp(0.0, 100.0);
            let used = ((1.0 - remaining_fraction) * 1000.0).round().max(0.0);
            if remaining_percentage <= 0.0 {
                limit_reached = true;
            }
            let reset_at = parse_reset_timestamp(quota_info.get("resetTime"));
            windows.push(GatewayQuotaWindow {
                name: model_key.to_owned(),
                used,
                total: 1000.0,
                remaining_percentage,
                reset_at,
                unlimited: false,
                unit: Some("req".to_owned()),
            });
        }
    }

    let summary_windows = parse_weekly_quota_summary(data);
    for w in summary_windows {
        if w.remaining_percentage <= 0.0 {
            limit_reached = true;
        }
        windows.push(w);
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
            .append_pair("client_id", ANTIGRAVITY_CLIENT_ID)
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
        let (parsed_opt, last_status) = self
            .post_token_with_candidates(
                "authorization_code",
                &[
                    ("code", code),
                    ("redirect_uri", redirect_uri),
                    ("code_verifier", code_verifier),
                ],
            )
            .await?;

        let Some(parsed) = parsed_opt else {
            return Err(GatewayError::OAuthExchangeFailed(format!(
                "gemini_token_http_{last_status}"
            )));
        };
        let email = self.fetch_email(&parsed.access_token).await;
        let ctx = self
            .fetch_code_assist_context(&parsed.access_token, true)
            .await;
        let now = crate::database::unix_seconds().unwrap_or(0);
        let expires_at = parsed.expires_in.map(|ttl| now + ttl.max(60));
        Ok(ExchangedOAuthAccount {
            bundle: OAuthTokenBundle {
                access_token: parsed.access_token,
                refresh_token: parsed.refresh_token,
                upstream_oauth_token: None,
                account_context_id: ctx.project_id,
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

        let (parsed_opt, _) = self
            .post_token_with_candidates("refresh_token", &[("refresh_token", refresh_token)])
            .await?;

        let Some(parsed) = parsed_opt else {
            return Err(GatewayError::AccountReauthRequired);
        };
        let existing_context = current
            .account_context_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        let account_context_id = match existing_context {
            Some(ctx) => Some(ctx),
            None => {
                self.fetch_code_assist_context(&parsed.access_token, true)
                    .await
                    .project_id
            }
        };
        let now = crate::database::unix_seconds().unwrap_or(0);
        let expires_at = parsed.expires_in.map(|ttl| now + ttl.max(60));
        Ok(ExchangedOAuthAccount {
            bundle: OAuthTokenBundle {
                access_token: parsed.access_token,
                refresh_token: parsed
                    .refresh_token
                    .or_else(|| current.refresh_token.clone()),
                upstream_oauth_token: None,
                account_context_id,
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
        let ctx = self
            .fetch_code_assist_context(&bundle.access_token, false)
            .await;
        let project_id = ctx
            .project_id
            .clone()
            .or_else(|| existing_project.clone());
        let discovered_to_persist = if existing_project.is_none() {
            project_id.clone()
        } else {
            None
        };

        let req_body = match project_id.as_deref() {
            Some(pid) if !pid.trim().is_empty() => json!({ "project": pid }),
            _ => json!({}),
        };

        let resp = self
            .http
            .post(&self.retrieve_quota_url)
            .bearer_auth(&bundle.access_token)
            .header("Accept", "application/json")
            .header("User-Agent", ANTIGRAVITY_USER_AGENT)
            .header("X-Client-Name", "antigravity")
            .header("X-Client-Version", ANTIGRAVITY_IDE_VERSION)
            .json(&req_body)
            .send()
            .await
            .map_err(|e| GatewayError::OAuthExchangeFailed(format!("gemini_quota_error: {e}")))?;

        let status = resp.status().as_u16();
        if status == 401 {
            return Err(GatewayError::AccountReauthRequired);
        }
        if status == 403 {
            let err_text = resp.text().await.unwrap_or_default();
            let detail = Self::extract_error_message(&err_text)
                .or(ctx.error_message)
                .unwrap_or_else(|| {
                    "Antigravity quota API access forbidden. Chat may still work.".to_owned()
                });
            return Ok(VendorQuotaSnapshot {
                plan: Some(ctx.plan.unwrap_or_else(|| "Free".to_owned())),
                limit_reached: false,
                windows: Vec::new(),
                message: Some(detail),
                discovered_account_context_id: discovered_to_persist,
            });
        }
        if !resp.status().is_success() {
            return Err(GatewayError::OAuthExchangeFailed(format!(
                "gemini_quota_http_{status}"
            )));
        }

        let payload: Value = resp
            .json()
            .await
            .map_err(|e| GatewayError::OAuthExchangeFailed(format!("gemini_quota_json: {e}")))?;

        let mut snapshot = parse_gemini_quota_payload_with_tier(
            &payload,
            ctx.plan,
            discovered_to_persist,
            ctx.is_free_tier,
        );

        let weekly_windows = self
            .fetch_weekly_summary_windows(&bundle.access_token, project_id.as_deref())
            .await;
        for w in weekly_windows {
            if !snapshot.windows.iter().any(|existing| existing.name == w.name) {
                if w.remaining_percentage <= 0.0 {
                    snapshot.limit_reached = true;
                }
                snapshot.windows.push(w);
            }
        }
        if snapshot.windows.is_empty() && snapshot.message.is_none() {
            snapshot.message = ctx.error_message;
        }
        Ok(snapshot)
    }

    fn extra_headers_for(&self, _bundle: &OAuthTokenBundle) -> Vec<(String, String)> {
        vec![("User-Agent".to_owned(), ANTIGRAVITY_USER_AGENT.to_owned())]
    }
}
