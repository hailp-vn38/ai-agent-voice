use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::{
    database::unix_seconds,
    providers::llm::gateway::muse::{
        cdp::{BrowserEngine, BrowserPage},
        config::{
            MuseAccountCredential, NEW_THREAD_URL, SESSION_API_URL, SITE_URL, VM_WAKE_API_URL,
            has_required_cookies, parse_accounts,
        },
    },
    services::llm_gateway::types::{
        GatewayError, GatewayQuotaWindow, GatewayVendor, OAuthTokenBundle, VendorQuotaSnapshot,
    },
};
#[cfg(not(test))]
use crate::providers::llm::gateway::muse::cdp::CdpBrowserEngine;

use super::{ExchangedOAuthAccount, OAuthVendorDriver};

const DEFAULT_MUSE_RENEWAL_INTERVAL_SECS: i64 = 6 * 3600;
#[cfg(not(test))]
const INTERACTIVE_LOGIN_CDP_PORT: u16 = 19223;

pub struct MuseOAuthDriver {
    http: reqwest::Client,
    session_api_url: String,
    vm_wake_api_url: String,
    verify_online: bool,
    login_engine: Option<Arc<dyn BrowserEngine>>,
    login_pages: Mutex<HashMap<String, Arc<dyn BrowserPage>>>,
    login_unavailable_states: Mutex<HashSet<String>>,
}

impl MuseOAuthDriver {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent(
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36",
            )
            .build()
            .unwrap_or_default();
        #[cfg(not(test))]
        let login_engine: Option<Arc<dyn BrowserEngine>> =
            Some(Arc::new(CdpBrowserEngine::new(INTERACTIVE_LOGIN_CDP_PORT, false)));
        #[cfg(test)]
        let login_engine: Option<Arc<dyn BrowserEngine>> = None;

        Self {
            http,
            session_api_url: SESSION_API_URL.to_owned(),
            vm_wake_api_url: VM_WAKE_API_URL.to_owned(),
            verify_online: !cfg!(test),
            login_engine,
            login_pages: Mutex::new(HashMap::new()),
            login_unavailable_states: Mutex::new(HashSet::new()),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_browser_engine(engine: Arc<dyn BrowserEngine>) -> Self {
        let mut drv = Self::new();
        drv.login_engine = Some(engine);
        drv.verify_online = false;
        drv
    }

    #[cfg(test)]
    pub(crate) fn with_test_endpoints(
        session_api_url: String,
        vm_wake_api_url: String,
    ) -> Self {
        let mut drv = Self::new();
        drv.session_api_url = session_api_url;
        drv.vm_wake_api_url = vm_wake_api_url;
        drv.verify_online = true;
        drv
    }

    async fn verify_and_renew_session(
        &self,
        mut cred: MuseAccountCredential,
        strict_network: bool,
    ) -> Result<(MuseAccountCredential, Option<String>), GatewayError> {
        if !self.verify_online {
            return Ok((cred, None));
        }

        let cookie_header = format_cookie_header(&cred.cookies);
        let resp = match self
            .http
            .get(&self.session_api_url)
            .header("Origin", SITE_URL)
            .header("Referer", NEW_THREAD_URL)
            .header("Accept", "application/json")
            .header("Cookie", &cookie_header)
            .send()
            .await
        {
            Ok(resp) => resp,
            Err(err) => {
                if strict_network {
                    return Err(GatewayError::OAuthExchangeFailed(format!(
                        "muse_session_unreachable: {err}"
                    )));
                }
                return Ok((cred, None));
            }
        };

        let status = resp.status().as_u16();
        if status == 401 || status == 403 {
            if strict_network {
                return Err(GatewayError::OAuthExchangeFailed(
                    "muse_session_unauthorized".to_owned(),
                ));
            }
            return Ok((cred, None));
        }
        if !resp.status().is_success() {
            if strict_network {
                return Err(GatewayError::OAuthExchangeFailed(format!(
                    "muse_session_http_{status}"
                )));
            }
            return Ok((cred, None));
        }

        for header_val in resp.headers().get_all(reqwest::header::SET_COOKIE) {
            if let Ok(raw) = header_val.to_str()
                && let Some(first_part) = raw.split(';').next()
                && let Some((k, v)) = first_part.split_once('=')
            {
                let k = k.trim();
                let v = v.trim();
                if !k.is_empty() && !v.is_empty() {
                    cred.cookies.insert(k.to_owned(), v.to_owned());
                }
            }
        }

        let mut discovered_email: Option<String> = None;
        if let Ok(body) = resp.json::<Value>().await {
            if cred.vm_id.is_none() {
                cred.vm_id = body
                    .get("vm_id")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(ToOwned::to_owned);
            }
            discovered_email = body
                .get("email")
                .and_then(Value::as_str)
                .or_else(|| {
                    body.get("user")
                        .and_then(|u| u.get("email"))
                        .and_then(Value::as_str)
                })
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToOwned::to_owned);
        }

        if let Some(vm) = &cred.vm_id {
            let updated_cookie_header = format_cookie_header(&cred.cookies);
            let _ = self
                .http
                .post(&self.vm_wake_api_url)
                .header("Origin", SITE_URL)
                .header("Referer", NEW_THREAD_URL)
                .header("Cookie", updated_cookie_header)
                .json(&serde_json::json!({
                    "vm_id": vm,
                    "retry_count": 0,
                    "connect_attempt_id": Uuid::new_v4().to_string(),
                }))
                .send()
                .await;
        }

        Ok((cred, discovered_email))
    }

    fn build_exchanged_account(
        cred: MuseAccountCredential,
        discovered_email: Option<String>,
    ) -> Result<ExchangedOAuthAccount, GatewayError> {
        let now_epoch = unix_seconds().unwrap_or(0);
        let expires_at = cred
            .cookie_expires
            .map(|exp| exp as i64)
            .filter(|&exp| exp > now_epoch)
            .or(Some(now_epoch + DEFAULT_MUSE_RENEWAL_INTERVAL_SECS));
        let vm_id = cred.vm_id.clone();
        let serialized_cred =
            serde_json::to_string(&cred).map_err(|_| GatewayError::InvalidTokenBundle)?;
        Ok(ExchangedOAuthAccount {
            bundle: OAuthTokenBundle {
                access_token: serialized_cred,
                refresh_token: Some("muse_cookie_session".to_owned()),
                upstream_oauth_token: None,
                account_context_id: vm_id,
            },
            account_email: discovered_email,
            expires_at,
        })
    }
}

impl Default for MuseOAuthDriver {
    fn default() -> Self {
        Self::new()
    }
}

fn format_cookie_header(cookies: &BTreeMap<String, String>) -> String {
    cookies
        .iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("; ")
}

#[async_trait]
impl OAuthVendorDriver for MuseOAuthDriver {
    fn vendor(&self) -> GatewayVendor {
        GatewayVendor::Muse
    }

    fn build_pkce_authorize_url(
        &self,
        state: &str,
        _code_challenge: &str,
        _redirect_uri: &str,
    ) -> Result<String, GatewayError> {
        let mut url =
            url::Url::parse(NEW_THREAD_URL).map_err(|_| GatewayError::OAuthCallbackInvalid)?;
        url.query_pairs_mut().append_pair("state", state);
        Ok(url.to_string())
    }

    async fn exchange_pkce_code(
        &self,
        code: &str,
        state: &str,
        _code_verifier: &str,
        _redirect_uri: &str,
    ) -> Result<ExchangedOAuthAccount, GatewayError> {
        // Clean up any open interactive CDP login tab for this session state.
        if let Some(page) = self.login_pages.lock().await.remove(state) {
            page.close().await;
            if let Some(engine) = &self.login_engine {
                engine
                    .dispose_context(&format!("muse-login-{state}"))
                    .await;
            }
        }

        let mut accounts = parse_accounts(code);
        if accounts.is_empty() {
            return Err(GatewayError::OAuthExchangeFailed(
                "missing_required_muse_cookies (hatch_sess, hatch_gw, hatch_vml, hatch_native_auth_device)".to_owned(),
            ));
        }
        let cred = accounts.remove(0);
        let (verified_cred, email) = self.verify_and_renew_session(cred, false).await?;
        Self::build_exchanged_account(verified_cred, email)
    }

    async fn poll_pkce_session(
        &self,
        state: &str,
    ) -> Result<Option<ExchangedOAuthAccount>, GatewayError> {
        let Some(engine) = &self.login_engine else {
            return Ok(None);
        };
        if self.login_unavailable_states.lock().await.contains(state) {
            return Ok(None);
        }

        let page = {
            let mut guard = self.login_pages.lock().await;
            if let Some(existing) = guard.get(state) {
                Arc::clone(existing)
            } else {
                let context_key = format!("muse-login-{state}");
                match engine.open_page(&context_key, &[]).await {
                    Ok(new_page) => {
                        if new_page.navigate(NEW_THREAD_URL).await.is_err() {
                            new_page.close().await;
                            engine.dispose_context(&context_key).await;
                            self.login_unavailable_states
                                .lock()
                                .await
                                .insert(state.to_owned());
                            return Ok(None);
                        }
                        guard.insert(state.to_owned(), Arc::clone(&new_page));
                        new_page
                    }
                    Err(_) => {
                        self.login_unavailable_states
                            .lock()
                            .await
                            .insert(state.to_owned());
                        return Ok(None);
                    }
                }
            }
        };

        let Ok(cookies) = page
            .get_cookies(&["https://muse.ai", "https://www.muse.ai"])
            .await
        else {
            return Ok(None);
        };

        if !has_required_cookies(&cookies) {
            return Ok(None);
        }

        if let Some(closed_page) = self.login_pages.lock().await.remove(state) {
            closed_page.close().await;
        }
        engine
            .dispose_context(&format!("muse-login-{state}"))
            .await;

        let cred = MuseAccountCredential {
            label: "muse-cdp".to_owned(),
            cookies,
            cookie_expires: None,
            vm_id: None,
        };
        let (verified_cred, email) = self.verify_and_renew_session(cred, false).await?;
        let exchanged = Self::build_exchanged_account(verified_cred, email)?;
        Ok(Some(exchanged))
    }

    async fn refresh_token(
        &self,
        current: &OAuthTokenBundle,
    ) -> Result<ExchangedOAuthAccount, GatewayError> {
        let mut accounts = parse_accounts(&current.access_token);
        if accounts.is_empty() {
            return Err(GatewayError::CredentialInvalid);
        }
        let mut cred = accounts.remove(0);
        if cred.vm_id.is_none() {
            cred.vm_id = current.account_context_id.clone();
        }
        let (verified_cred, email) = self.verify_and_renew_session(cred, true).await?;
        Self::build_exchanged_account(verified_cred, email)
    }

    async fn fetch_quota(
        &self,
        bundle: &OAuthTokenBundle,
    ) -> Result<VendorQuotaSnapshot, GatewayError> {
        let mut accounts = parse_accounts(&bundle.access_token);
        if accounts.is_empty() {
            return Err(GatewayError::CredentialInvalid);
        }
        let mut cred = accounts.remove(0);
        if cred.vm_id.is_none() {
            cred.vm_id = bundle.account_context_id.clone();
        }
        let (verified_cred, _) = self.verify_and_renew_session(cred, false).await?;
        Ok(VendorQuotaSnapshot {
            plan: Some("Muse AI (CDP)".to_owned()),
            limit_reached: false,
            windows: vec![GatewayQuotaWindow {
                name: "Session".to_owned(),
                used: 0.0,
                total: 100.0,
                remaining_percentage: 100.0,
                reset_at: None,
                unlimited: true,
                unit: Some("CDP".to_owned()),
            }],
            message: verified_cred
                .vm_id
                .as_ref()
                .map(|vm| format!("VM: {vm}")),
            discovered_account_context_id: verified_cred.vm_id,
        })
    }

    fn extra_headers_for(&self, _bundle: &OAuthTokenBundle) -> Vec<(String, String)> {
        Vec::new()
    }
}
