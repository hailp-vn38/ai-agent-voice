use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex;
use uuid::Uuid;

#[cfg(not(test))]
use crate::providers::llm::gateway::muse::cdp::CdpBrowserEngine;
use crate::{
    database::unix_seconds,
    providers::llm::gateway::muse::{
        cdp::{BrowserEngine, BrowserPage, extract_wss_via_chromium},
        config::{
            MuseAccountCredential, NEW_THREAD_URL, SESSION_API_URL, SITE_URL, VM_WAKE_API_URL,
            format_cookie_header, has_required_cookies, parse_accounts, parse_cookie_header,
        },
        credentials::{
            WSS_EXPIRY_BUFFER_SECS, build_wss_url, build_wss_url_from_init_json,
            fetch_wss_from_cookie_http, get_wss_expiry_unix_secs, is_wss_valid, merge_set_cookies,
            parse_session_json,
        },
        pool::UpstreamFailureKind,
    },
    services::llm_gateway::types::{
        GatewayError, GatewayQuotaWindow, GatewayVendor, OAuthTokenBundle, VendorQuotaSnapshot,
    },
};

use super::{ExchangedOAuthAccount, OAuthVendorDriver};

const DEFAULT_MUSE_RENEWAL_INTERVAL_SECS: i64 = 6 * 3600;
#[cfg(not(test))]
const HEADLESS_REFRESH_CDP_PORT: u16 = 19222;
#[cfg(not(test))]
const INTERACTIVE_LOGIN_CDP_PORT: u16 = 19223;

pub struct MuseOAuthDriver {
    http: reqwest::Client,
    session_api_url: String,
    vm_wake_api_url: String,
    verify_online: bool,
    headless_engine: Option<Arc<dyn BrowserEngine>>,
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
        let headless_engine: Option<Arc<dyn BrowserEngine>> = Some(Arc::new(
            CdpBrowserEngine::new(HEADLESS_REFRESH_CDP_PORT, true),
        ));
        #[cfg(test)]
        let headless_engine: Option<Arc<dyn BrowserEngine>> = None;
        #[cfg(not(test))]
        let login_engine: Option<Arc<dyn BrowserEngine>> = Some(Arc::new(CdpBrowserEngine::new(
            INTERACTIVE_LOGIN_CDP_PORT,
            false,
        )));
        #[cfg(test)]
        let login_engine: Option<Arc<dyn BrowserEngine>> = None;

        Self {
            http,
            session_api_url: SESSION_API_URL.to_owned(),
            vm_wake_api_url: VM_WAKE_API_URL.to_owned(),
            verify_online: !cfg!(test),
            headless_engine,
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
    pub(crate) fn with_test_endpoints(session_api_url: String, vm_wake_api_url: String) -> Self {
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
        if !self.verify_online || cred.cookies.is_empty() {
            return Ok((cred, None));
        }

        let previous_valid_wss = cred
            .wss_url
            .clone()
            .filter(|u| is_wss_valid(u, WSS_EXPIRY_BUFFER_SECS))
            .map(|u| (u, cred.wss_expires));
        let wss_needs_renewal = strict_network || previous_valid_wss.is_none();
        if wss_needs_renewal {
            cred.wss_url = None;
            cred.wss_expires = None;
        }

        let site_base_url = self
            .session_api_url
            .split("/api/")
            .next()
            .unwrap_or(SITE_URL)
            .trim_end_matches('/');

        let mut session_verified = false;
        let mut discovered_email: Option<String> = None;
        let mut endpoint_url: Option<String> = None;

        // Tier 1: Direct Meta Muse Session REST API (/api/session -> /api/hatch/token)
        let cookie_header = format_cookie_header(&cred.cookies);
        if let Ok(resp) = self
            .http
            .get(&self.session_api_url)
            .header("Origin", site_base_url)
            .header("Referer", NEW_THREAD_URL)
            .header("Accept", "application/json")
            .header("Cookie", &cookie_header)
            .send()
            .await
        {
            let status = resp.status().as_u16();
            let merged_header = merge_set_cookies(&cookie_header, resp.headers());
            for (k, v) in parse_cookie_header(&merged_header) {
                cred.cookies.insert(k, v);
            }
            if status == 401 {
                if strict_network {
                    return Err(GatewayError::AccountReauthRequired);
                }
                return Ok((cred, None));
            }
            if resp.status().is_success() {
                session_verified = true;
                if let Ok(body) = resp.json::<Value>().await {
                    let (parsed_vm, parsed_ep, parsed_email) = parse_session_json(&body);
                    if parsed_vm.is_some() {
                        cred.vm_id = parsed_vm;
                    }
                    endpoint_url = parsed_ep.or_else(|| {
                        cred.vm_id
                            .as_ref()
                            .map(|vm| format!("wss://{vm}.metaaivm.com/"))
                    });
                    discovered_email = parsed_email;
                }
            }
        }

        if let (Some(vm), Some(ep)) = (cred.vm_id.clone(), endpoint_url)
            && cred.wss_url.is_none()
        {
            for t_attempt in 0..2 {
                let updated_cookie_header = format_cookie_header(&cred.cookies);
                if let Ok(t_resp) = self
                    .http
                    .post(format!("{site_base_url}/api/hatch/token"))
                    .header("Origin", site_base_url)
                    .header("Referer", NEW_THREAD_URL)
                    .header("Cookie", &updated_cookie_header)
                    .json(&serde_json::json!({
                        "vmAddress": ep,
                        "vmName": vm,
                    }))
                    .send()
                    .await
                {
                    let token_merged_cookies =
                        merge_set_cookies(&updated_cookie_header, t_resp.headers());
                    for (k, v) in parse_cookie_header(&token_merged_cookies) {
                        cred.cookies.insert(k, v);
                    }
                    if t_resp.status().is_success()
                        && let Ok(t_body) = t_resp.json::<Value>().await
                        && let Some(auth_token) = t_body
                            .get("token")
                            .or_else(|| t_body.get("authToken"))
                            .and_then(Value::as_str)
                            .filter(|s| !s.is_empty())
                    {
                        let notary_token = t_body
                            .get("notary_token")
                            .or_else(|| t_body.get("noiseNotaryToken"))
                            .or_else(|| t_body.get("notaryToken"))
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        let wss = build_wss_url(&vm, auth_token, notary_token);
                        cred.wss_expires = get_wss_expiry_unix_secs(&wss);
                        cred.wss_url = Some(wss);
                        break;
                    }
                }
                if t_attempt == 0 {
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            }
        }

        // Tier 2: Multi-tier HTTP/SSR probe (`GET /api/session?vm_state=live` -> `GET /` RSC stream)
        if cred.wss_url.is_none() {
            let current_cookie_header = format_cookie_header(&cred.cookies);
            match fetch_wss_from_cookie_http(&self.http, &current_cookie_header, site_base_url)
                .await
            {
                Ok(bootstrapped) => {
                    session_verified = true;
                    for (k, v) in parse_cookie_header(&bootstrapped.cookie_header) {
                        cred.cookies.insert(k, v);
                    }
                    if bootstrapped.vm_id.is_some() {
                        cred.vm_id = bootstrapped.vm_id;
                    }
                    if discovered_email.is_none() {
                        discovered_email = bootstrapped.discovered_email;
                    }
                    cred.wss_expires = get_wss_expiry_unix_secs(&bootstrapped.wss_url);
                    cred.wss_url = Some(bootstrapped.wss_url);
                }
                Err((UpstreamFailureKind::AuthInvalid, _)) if !session_verified => {
                    if strict_network {
                        return Err(GatewayError::AccountReauthRequired);
                    }
                    return Ok((cred, None));
                }
                Err(_) => {}
            }
        }

        // Tier 3: Headless Chromium CDP fallback when HTTP/SSR is blocked by Cloudflare
        if cred.wss_url.is_none()
            && let Some(engine) = self.headless_engine.as_ref().or(self.login_engine.as_ref())
        {
            let ctx_key = format!("muse-refresh-{}", Uuid::new_v4());
            match extract_wss_via_chromium(&**engine, &ctx_key, &cred.cookies).await {
                Ok(bootstrapped) => {
                    session_verified = true;
                    for (k, v) in parse_cookie_header(&bootstrapped.cookie_header) {
                        cred.cookies.insert(k, v);
                    }
                    if bootstrapped.vm_id.is_some() {
                        cred.vm_id = bootstrapped.vm_id;
                    }
                    if discovered_email.is_none() {
                        discovered_email = bootstrapped.discovered_email;
                    }
                    cred.wss_expires = get_wss_expiry_unix_secs(&bootstrapped.wss_url);
                    cred.wss_url = Some(bootstrapped.wss_url);
                }
                Err((UpstreamFailureKind::AuthInvalid, _)) if !session_verified => {
                    if strict_network {
                        return Err(GatewayError::AccountReauthRequired);
                    }
                }
                Err(_) => {}
            }
        }

        if cred.wss_url.is_none()
            && let Some((prev_wss, prev_exp)) = previous_valid_wss
        {
            cred.wss_url = Some(prev_wss);
            cred.wss_expires = prev_exp;
        }

        if strict_network && !session_verified && cred.wss_url.is_none() {
            return Err(GatewayError::OAuthExchangeFailed(
                "muse_session_renewal_failed".to_owned(),
            ));
        }

        if let Some(vm) = &cred.vm_id {
            let updated_cookie_header = format_cookie_header(&cred.cookies);
            let _ = self
                .http
                .post(&self.vm_wake_api_url)
                .header("Origin", site_base_url)
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

    pub(crate) fn build_exchanged_account(
        cred: MuseAccountCredential,
        discovered_email: Option<String>,
    ) -> Result<ExchangedOAuthAccount, GatewayError> {
        let now_epoch = unix_seconds().unwrap_or(0);
        let candidate_exp = match (cred.cookie_expires, cred.wss_expires) {
            (Some(c), Some(w)) => Some(c.min(w)),
            (Some(c), None) => Some(c),
            (None, Some(w)) => Some(w),
            (None, None) => None,
        };
        let expires_at = candidate_exp
            .map(|exp| exp as i64)
            .filter(|&exp| exp > 0)
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
                engine.dispose_context(&format!("muse-login-{state}")).await;
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

        let mut init_wss_url = None;
        let mut init_vm_id = None;
        if let Ok(val) = page
            .evaluate_json(
                "window.__HATCH_GATEWAY_INIT__ || null",
                false,
                Duration::from_secs(3),
            )
            .await
            && val.is_object()
            && let Ok(wss) = build_wss_url_from_init_json(&val)
        {
            init_vm_id = val
                .get("vmName")
                .or_else(|| val.get("vmId"))
                .or_else(|| val.get("vm_id"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(ToOwned::to_owned);
            init_wss_url = Some(wss);
        }

        if let Some(closed_page) = self.login_pages.lock().await.remove(state) {
            closed_page.close().await;
        }
        engine.dispose_context(&format!("muse-login-{state}")).await;

        let wss_expires = init_wss_url.as_deref().and_then(get_wss_expiry_unix_secs);
        let cred = MuseAccountCredential {
            label: "muse-cdp".to_owned(),
            cookies,
            cookie_expires: None,
            vm_id: init_vm_id,
            wss_url: init_wss_url,
            wss_expires,
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
        let now_epoch = unix_seconds().unwrap_or(0);
        if cred
            .cookie_expires
            .is_some_and(|exp| (exp as i64) <= now_epoch)
        {
            cred.cookie_expires = None;
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
            message: verified_cred.vm_id.as_ref().map(|vm| format!("VM: {vm}")),
            discovered_account_context_id: verified_cred.vm_id,
        })
    }

    fn extra_headers_for(&self, _bundle: &OAuthTokenBundle) -> Vec<(String, String)> {
        Vec::new()
    }

    fn proactive_refresh_margin_secs(&self, default_margin_secs: i64) -> i64 {
        default_margin_secs.max(1800)
    }

    fn can_self_heal_reauth(&self, bundle: &OAuthTokenBundle, last_error: Option<&str>) -> bool {
        last_error != Some("account_reauth_required")
            && parse_accounts(&bundle.access_token)
                .first()
                .is_some_and(|c| has_required_cookies(&c.cookies))
    }

    fn is_transient_refresh_error(&self, err: &GatewayError) -> bool {
        matches!(err, GatewayError::OAuthExchangeFailed(_))
    }
}
