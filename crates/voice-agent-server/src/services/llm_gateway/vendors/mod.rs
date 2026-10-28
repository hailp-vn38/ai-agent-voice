pub mod anthropic;
pub mod gemini;
pub mod github_copilot;
pub mod openai_codex;

use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;

use super::types::{GatewayError, GatewayVendor, OAuthTokenBundle};

#[derive(Debug, Clone)]
pub struct ExchangedOAuthAccount {
    pub bundle: OAuthTokenBundle,
    pub account_email: Option<String>,
    pub expires_at: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct DeviceCodeGrant {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub interval_secs: u64,
    pub expires_in_secs: u64,
}

#[derive(Debug, Clone)]
pub enum DevicePollOutcome {
    Pending { next_interval_secs: Option<u64> },
    Completed(ExchangedOAuthAccount),
    Failed(String),
}

/// Internal Seam encapsulating the 4 external OAuth 2.0 providers.
#[async_trait]
pub trait OAuthVendorDriver: Send + Sync {
    fn vendor(&self) -> GatewayVendor;

    fn default_redirect_uri(&self) -> &'static str {
        "http://localhost:54545/callback"
    }

    fn build_pkce_authorize_url(
        &self,
        _state: &str,
        _code_challenge: &str,
        _redirect_uri: &str,
    ) -> Result<String, GatewayError> {
        Err(GatewayError::OAuthExchangeFailed(
            "pkce_not_supported_for_vendor".to_owned(),
        ))
    }

    async fn exchange_pkce_code(
        &self,
        _code: &str,
        _state: &str,
        _code_verifier: &str,
        _redirect_uri: &str,
    ) -> Result<ExchangedOAuthAccount, GatewayError> {
        Err(GatewayError::OAuthExchangeFailed(
            "pkce_not_supported_for_vendor".to_owned(),
        ))
    }

    async fn start_device_flow(&self) -> Result<DeviceCodeGrant, GatewayError> {
        Err(GatewayError::OAuthExchangeFailed(
            "device_flow_not_supported_for_vendor".to_owned(),
        ))
    }

    async fn poll_device_flow(&self, _device_code: &str) -> Result<DevicePollOutcome, GatewayError> {
        Err(GatewayError::OAuthExchangeFailed(
            "device_flow_not_supported_for_vendor".to_owned(),
        ))
    }

    async fn refresh_token(
        &self,
        current: &OAuthTokenBundle,
    ) -> Result<ExchangedOAuthAccount, GatewayError>;

    fn extra_headers_for(&self, bundle: &OAuthTokenBundle) -> Vec<(String, String)>;
}

pub fn default_vendor_drivers() -> BTreeMap<GatewayVendor, Arc<dyn OAuthVendorDriver>> {
    let mut map: BTreeMap<GatewayVendor, Arc<dyn OAuthVendorDriver>> = BTreeMap::new();
    map.insert(
        GatewayVendor::Anthropic,
        Arc::new(anthropic::AnthropicOAuthDriver::new()),
    );
    map.insert(
        GatewayVendor::OpenaiCodex,
        Arc::new(openai_codex::OpenAiCodexOAuthDriver::new()),
    );
    map.insert(
        GatewayVendor::Gemini,
        Arc::new(gemini::GeminiOAuthDriver::new()),
    );
    map.insert(
        GatewayVendor::GithubCopilot,
        Arc::new(github_copilot::GithubCopilotOAuthDriver::new()),
    );
    map
}

/// Deterministic in-memory fake driver used in `LlmGatewayHub` integration tests.
pub struct FakeOAuthVendorDriver {
    vendor: GatewayVendor,
    refresh_calls: AtomicUsize,
    device_ready: Mutex<bool>,
    next_email: Mutex<Option<String>>,
    next_expires_at: Mutex<Option<i64>>,
}

impl FakeOAuthVendorDriver {
    pub fn new(vendor: GatewayVendor) -> Self {
        Self {
            vendor,
            refresh_calls: AtomicUsize::new(0),
            device_ready: Mutex::new(false),
            next_email: Mutex::new(Some(format!("{}@example.com", vendor.as_str()))),
            next_expires_at: Mutex::new(Some(4_102_444_800)), // 2100-01-01
        }
    }

    pub fn set_device_ready(&self, ready: bool) {
        *self.device_ready.lock().expect("device_ready mutex") = ready;
    }

    pub fn set_next_expires_at(&self, expires_at: Option<i64>) {
        *self.next_expires_at.lock().expect("next_expires_at mutex") = expires_at;
    }

    pub fn refresh_call_count(&self) -> usize {
        self.refresh_calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl OAuthVendorDriver for FakeOAuthVendorDriver {
    fn vendor(&self) -> GatewayVendor {
        self.vendor
    }

    fn build_pkce_authorize_url(
        &self,
        state: &str,
        code_challenge: &str,
        redirect_uri: &str,
    ) -> Result<String, GatewayError> {
        let mut url = url::Url::parse(&format!(
            "https://auth.{}.test/oauth/authorize",
            self.vendor.as_str()
        ))
        .map_err(|_| GatewayError::OAuthCallbackInvalid)?;
        url.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("state", state)
            .append_pair("code_challenge", code_challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("redirect_uri", redirect_uri);
        Ok(url.to_string())
    }

    async fn exchange_pkce_code(
        &self,
        code: &str,
        _state: &str,
        code_verifier: &str,
        _redirect_uri: &str,
    ) -> Result<ExchangedOAuthAccount, GatewayError> {
        if code == "invalid_code" || code_verifier.is_empty() {
            return Err(GatewayError::OAuthExchangeFailed("invalid_grant".to_owned()));
        }
        let email = self.next_email.lock().expect("next_email").clone();
        let expires_at = *self.next_expires_at.lock().expect("next_expires_at");
        Ok(ExchangedOAuthAccount {
            bundle: OAuthTokenBundle {
                access_token: format!("tok_{}_{}", self.vendor.as_str(), code),
                refresh_token: Some(format!("ref_{}_{}", self.vendor.as_str(), code)),
                upstream_oauth_token: None,
                account_context_id: (self.vendor == GatewayVendor::OpenaiCodex)
                    .then(|| "acct_codex_test".to_owned()),
            },
            account_email: email,
            expires_at,
        })
    }

    async fn start_device_flow(&self) -> Result<DeviceCodeGrant, GatewayError> {
        Ok(DeviceCodeGrant {
            device_code: "dev_code_copilot_123".to_owned(),
            user_code: "ABCD-1234".to_owned(),
            verification_uri: "https://github.com/login/device".to_owned(),
            interval_secs: 5,
            expires_in_secs: 900,
        })
    }

    async fn poll_device_flow(&self, device_code: &str) -> Result<DevicePollOutcome, GatewayError> {
        if device_code != "dev_code_copilot_123" {
            return Ok(DevicePollOutcome::Failed("bad_device_code".to_owned()));
        }
        let ready = *self.device_ready.lock().expect("device_ready");
        if !ready {
            return Ok(DevicePollOutcome::Pending {
                next_interval_secs: Some(5),
            });
        }
        let expires_at = *self.next_expires_at.lock().expect("next_expires_at");
        Ok(DevicePollOutcome::Completed(ExchangedOAuthAccount {
            bundle: OAuthTokenBundle {
                access_token: "tid_copilot_bearer_token_01".to_owned(),
                refresh_token: None,
                upstream_oauth_token: Some("ghu_github_oauth_token_01".to_owned()),
                account_context_id: None,
            },
            account_email: Some("copilot@github.test".to_owned()),
            expires_at,
        }))
    }

    async fn refresh_token(
        &self,
        current: &OAuthTokenBundle,
    ) -> Result<ExchangedOAuthAccount, GatewayError> {
        let seq = self.refresh_calls.fetch_add(1, Ordering::SeqCst) + 1;
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        Ok(ExchangedOAuthAccount {
            bundle: OAuthTokenBundle {
                access_token: format!("tok_refreshed_{}_{seq}", self.vendor.as_str()),
                refresh_token: current
                    .refresh_token
                    .clone()
                    .or_else(|| Some(format!("ref_{}_{seq}", self.vendor.as_str()))),
                upstream_oauth_token: current.upstream_oauth_token.clone(),
                account_context_id: current.account_context_id.clone(),
            },
            account_email: None,
            expires_at: Some(4_102_444_800),
        })
    }

    fn extra_headers_for(&self, bundle: &OAuthTokenBundle) -> Vec<(String, String)> {
        match self.vendor {
            GatewayVendor::Anthropic => vec![
                ("anthropic-version".to_owned(), "2023-06-01".to_owned()),
                ("anthropic-beta".to_owned(), "oauth-2025-04-20".to_owned()),
            ],
            GatewayVendor::OpenaiCodex => bundle
                .account_context_id
                .as_ref()
                .map(|id| vec![("ChatGPT-Account-Id".to_owned(), id.clone())])
                .unwrap_or_default(),
            GatewayVendor::Gemini => Vec::new(),
            GatewayVendor::GithubCopilot => vec![
                ("Editor-Version".to_owned(), "vscode/1.96.0".to_owned()),
                ("Copilot-Integration-Id".to_owned(), "vscode-chat".to_owned()),
            ],
        }
    }
}
