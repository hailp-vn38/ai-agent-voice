use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use serde_json::Value;
use uuid::Uuid;

#[cfg(not(test))]
use crate::providers::llm::gateway::muse::cdp::CdpBrowserEngine;
use crate::{
    database::unix_seconds,
    providers::llm::gateway::muse::{
        cdp::{BrowserEngine, extract_wss_via_chromium},
        config::{
            MuseAccountCredential, NEW_THREAD_URL, SESSION_API_URL, SITE_URL, VM_WAKE_API_URL,
            format_cookie_header, has_required_cookies, parse_accounts, parse_cookie_header,
        },
        credentials::{
            WSS_EXPIRY_BUFFER_SECS, fetch_wss_from_cookie_http, get_wss_expiry_unix_secs,
            is_wss_valid, merge_set_cookies, parse_session_json, request_hatch_token_with_retry,
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

/// How `verify_and_renew_session` treats an upstream that rejects or cannot renew the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionCheck {
    /// Keep the stored credential on any failure (quota reads).
    Lenient,
    /// Reject cookies upstream reports as unauthorized, but tolerate an unreachable upstream so a
    /// paste still succeeds behind Cloudflare and renews later.
    RejectUnauthorized,
    /// Force a fresh WSS URL and fail when none can be obtained (token refresh).
    Strict,
}

pub struct MuseOAuthDriver {
    http: reqwest::Client,
    session_api_url: String,
    vm_wake_api_url: String,
    verify_online: bool,
    headless_engine: Option<Arc<dyn BrowserEngine>>,
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

        Self {
            http,
            session_api_url: SESSION_API_URL.to_owned(),
            vm_wake_api_url: VM_WAKE_API_URL.to_owned(),
            verify_online: !cfg!(test),
            headless_engine,
        }
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
        check: SessionCheck,
    ) -> Result<(MuseAccountCredential, Option<String>), GatewayError> {
        if !self.verify_online {
            return Ok((cred, None));
        }
        if cred.cookies.is_empty() {
            if check == SessionCheck::Strict {
                return Err(GatewayError::AccountReauthRequired);
            }
            return Ok((cred, None));
        }

        let wss_needs_renewal = check == SessionCheck::Strict
            || cred
                .wss_url
                .as_deref()
                .is_none_or(|u| !is_wss_valid(u, WSS_EXPIRY_BUFFER_SECS));
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
                if check != SessionCheck::Lenient {
                    return Err(GatewayError::AccountReauthRequired);
                }
                return Ok((cred, None));
            }
            if resp.status().is_success() {
                session_verified = true;
                if let Ok(body) = resp.json::<Value>().await {
                    let parsed = parse_session_json(&body);
                    if parsed.vm_id.is_some() {
                        cred.vm_id = parsed.vm_id;
                    }
                    endpoint_url = parsed.endpoint_url.or_else(|| {
                        cred.vm_id
                            .as_ref()
                            .map(|vm| format!("wss://{vm}.metaaivm.com/"))
                    });
                    discovered_email = parsed.discovered_email;
                }
            }
        }

        if let (Some(vm), Some(ep)) = (cred.vm_id.clone(), endpoint_url)
            && cred.wss_url.is_none()
        {
            let mut updated_cookie_header = format_cookie_header(&cred.cookies);
            if let Some(wss) = request_hatch_token_with_retry(
                &self.http,
                site_base_url,
                &mut updated_cookie_header,
                &vm,
                &ep,
            )
            .await
            {
                for (k, v) in parse_cookie_header(&updated_cookie_header) {
                    cred.cookies.insert(k, v);
                }
                cred.wss_expires = get_wss_expiry_unix_secs(&wss);
                cred.wss_url = Some(wss);
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
                    let _ = bootstrapped.apply_to(&mut cred, &mut discovered_email);
                }
                Err((UpstreamFailureKind::AuthInvalid, _)) if !session_verified => {
                    if check != SessionCheck::Lenient {
                        return Err(GatewayError::AccountReauthRequired);
                    }
                    return Ok((cred, None));
                }
                Err(_) => {}
            }
        }

        // Tier 3: Headless Chromium CDP fallback when HTTP/SSR is blocked by Cloudflare
        if cred.wss_url.is_none()
            && let Some(engine) = self.headless_engine.as_ref()
        {
            let ctx_key = format!("muse-refresh-{}", Uuid::new_v4());
            match extract_wss_via_chromium(&**engine, &ctx_key, &cred.cookies).await {
                Ok(bootstrapped) => {
                    let _ = bootstrapped.apply_to(&mut cred, &mut discovered_email);
                }
                Err((UpstreamFailureKind::AuthInvalid, _)) if !session_verified => {
                    if check != SessionCheck::Lenient {
                        return Err(GatewayError::AccountReauthRequired);
                    }
                }
                Err(_) => {}
            }
        }

        if check == SessionCheck::Strict && cred.wss_url.is_none() {
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
        mut cred: MuseAccountCredential,
        discovered_email: Option<String>,
    ) -> Result<ExchangedOAuthAccount, GatewayError> {
        let now_epoch = unix_seconds().unwrap_or(0);
        if cred.wss_expires.is_some()
            && cred
                .cookie_expires
                .is_some_and(|exp| (exp as i64) <= now_epoch)
        {
            cred.cookie_expires = None;
        }
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

    async fn import_cookie_session(
        &self,
        raw_cookies: &str,
    ) -> Result<ExchangedOAuthAccount, GatewayError> {
        let mut accounts = parse_accounts(raw_cookies);
        if accounts.is_empty() {
            return Err(GatewayError::OAuthExchangeFailed(
                "missing_required_muse_cookies (hatch_sess, hatch_gw, hatch_vml, hatch_native_auth_device)".to_owned(),
            ));
        }
        let cred = accounts.remove(0);
        let (verified_cred, email) = self
            .verify_and_renew_session(cred, SessionCheck::RejectUnauthorized)
            .await
            .map_err(|err| match err {
                GatewayError::AccountReauthRequired => {
                    GatewayError::OAuthExchangeFailed("muse_cookie_unauthorized".to_owned())
                }
                other => other,
            })?;
        Self::build_exchanged_account(verified_cred, email)
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
        let (verified_cred, email) = self
            .verify_and_renew_session(cred, SessionCheck::Strict)
            .await?;
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
        let (verified_cred, _) = self
            .verify_and_renew_session(cred, SessionCheck::Lenient)
            .await?;
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
        last_error != Some(GatewayError::AccountReauthRequired.code())
            && parse_accounts(&bundle.access_token)
                .first()
                .is_some_and(|c| has_required_cookies(&c.cookies))
    }

    fn is_transient_refresh_error(&self, err: &GatewayError) -> bool {
        matches!(err, GatewayError::OAuthExchangeFailed(_))
    }
}
