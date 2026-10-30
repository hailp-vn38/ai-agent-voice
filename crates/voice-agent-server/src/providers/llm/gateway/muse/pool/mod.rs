mod account;
mod lease;
mod types;
mod warmup;

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Instant;

use tokio::sync::Notify;
use uuid::Uuid;

use self::account::{AccountStatus, MuseAccountState};
pub(crate) use self::lease::VmOperationLease;
pub(crate) use self::types::{ConversationTurn, HotConversation, UpstreamFailureKind};
use super::cdp::{BrowserEngine, extract_wss_via_chromium};
use super::client::{MuseVmClient, MuseVmTransport, VmError, WsNoiseVmTransport};
use super::config::{
    MuseAccountCredential, SITE_URL, format_cookie_header, parse_accounts, parse_cookie_header,
};
use super::credentials::{
    WSS_EXPIRY_BUFFER_SECS, fetch_wss_from_cookie_http, get_wss_expiry_unix_secs, is_wss_valid,
};

pub(super) struct PoolState {
    pub(super) accounts: Vec<MuseAccountState>,
    pub(super) rr_cursor: usize,
}

pub(super) struct MusePoolInner {
    pub http: reqwest::Client,
    pub transport: Arc<dyn MuseVmTransport>,
    pub has_custom_transport: bool,
    pub fallback_browser: Option<Arc<dyn BrowserEngine>>,
    pub state: Mutex<PoolState>,
    pub notify: Notify,
}

#[derive(Clone)]
pub(crate) struct SharedMusePool {
    pub(super) inner: Arc<MusePoolInner>,
}

static GLOBAL_POOLS: LazyLock<Mutex<HashMap<String, Arc<SharedMusePool>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

impl SharedMusePool {
    pub fn for_gateway_lease(
        http: &reqwest::Client,
        account_id: &str,
        credential_json: &str,
        custom_transport: Option<Arc<dyn MuseVmTransport>>,
    ) -> Arc<Self> {
        let mut credentials = parse_accounts(credential_json);
        if !credentials.is_empty() {
            credentials.truncate(1);
            credentials[0].label = account_id.to_string();
        }

        let transport_id = custom_transport
            .as_ref()
            .map(|t| Arc::as_ptr(t) as *const () as usize)
            .unwrap_or(0);
        let cache_key = format!("gw:{account_id}:{transport_id}:{credential_json}");
        let mut map = GLOBAL_POOLS.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(existing) = map.get(&cache_key) {
            return Arc::clone(existing);
        }
        map.retain(|k, _| !k.starts_with(&format!("gw:{account_id}:{transport_id}:")));
        let has_custom = custom_transport.is_some();
        let transport: Arc<dyn MuseVmTransport> =
            custom_transport.unwrap_or_else(|| Arc::new(WsNoiseVmTransport));
        let pool = Arc::new(Self::new_with_options(
            http.clone(),
            transport,
            has_custom,
            None,
            credentials,
        ));
        map.insert(cache_key, Arc::clone(&pool));
        pool
    }

    fn new_with_options(
        http: reqwest::Client,
        transport: Arc<dyn MuseVmTransport>,
        has_custom_transport: bool,
        fallback_browser: Option<Arc<dyn BrowserEngine>>,
        credentials: Vec<MuseAccountCredential>,
    ) -> Self {
        let accounts: Vec<MuseAccountState> = credentials
            .into_iter()
            .map(MuseAccountState::new)
            .collect();
        let has_accounts = !accounts.is_empty();
        let pool = Self {
            inner: Arc::new(MusePoolInner {
                http,
                transport,
                has_custom_transport,
                fallback_browser,
                state: Mutex::new(PoolState {
                    accounts,
                    rr_cursor: 0,
                }),
                notify: Notify::new(),
            }),
        };
        if has_accounts && !has_custom_transport {
            pool.trigger_account_warmup(0);
        }
        pool
    }

    pub async fn acquire_for_chat(
        &self,
        prior_turns: &[ConversationTurn],
    ) -> Result<VmOperationLease, (UpstreamFailureKind, String)> {
        self.acquire_internal(prior_turns, true).await
    }

    pub async fn acquire_fresh(&self) -> Result<VmOperationLease, (UpstreamFailureKind, String)> {
        self.acquire_internal(&[], false).await
    }

    async fn acquire_internal(
        &self,
        prior_turns: &[ConversationTurn],
        prefer_hot: bool,
    ) -> Result<VmOperationLease, (UpstreamFailureKind, String)> {
        loop {
            let candidate = {
                let mut state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
                let now = Instant::now();
                let count = state.accounts.len();
                if count == 0 {
                    return Err((
                        UpstreamFailureKind::AuthInvalid,
                        "gateway muse account is missing a valid hatch_sess cookie or wss:// URL"
                            .to_string(),
                    ));
                }

                let mut all_auth_failed = true;
                let mut any_healthy = false;
                let mut shortest_cooldown: Option<(UpstreamFailureKind, std::time::Duration)> =
                    None;

                for acc in &mut state.accounts {
                    if acc.is_healthy(now) {
                        any_healthy = true;
                        all_auth_failed = false;
                    } else {
                        if !matches!(acc.status, AccountStatus::AuthFailed) {
                            all_auth_failed = false;
                        }
                        if let Some((reason, rem)) = acc.cooldown_remaining(now)
                            && shortest_cooldown.is_none_or(|(_, best)| rem < best)
                        {
                            shortest_cooldown = Some((reason, rem));
                        }
                    }
                }

                if !any_healthy {
                    if all_auth_failed {
                        return Err((
                            UpstreamFailureKind::AuthInvalid,
                            "all Muse accounts failed authentication (session expired or invalid)"
                                .to_string(),
                        ));
                    }
                    if let Some((reason, rem)) = shortest_cooldown {
                        return Err((
                            reason,
                            format!(
                                "all Muse accounts are in cooldown ({reason:?}, retry in {}s)",
                                rem.as_secs().max(1)
                            ),
                        ));
                    }
                    return Err((
                        UpstreamFailureKind::Transient,
                        "no healthy Muse accounts available".to_string(),
                    ));
                }

                let mut chosen: Option<(usize, Option<HotConversation>)> = None;

                if prefer_hot && !prior_turns.is_empty() {
                    for idx in 0..count {
                        let acc = &mut state.accounts[idx];
                        if acc.is_healthy(now)
                            && acc.has_capacity()
                            && let Some(hot) = acc.take_matching_hot_session(prior_turns, now)
                        {
                            acc.active_ops += 1;
                            chosen = Some((idx, Some(hot)));
                            break;
                        }
                    }
                }

                if chosen.is_none() {
                    let start = state.rr_cursor % count;
                    for offset in 0..count {
                        let idx = (start + offset) % count;
                        let acc = &mut state.accounts[idx];
                        if acc.is_healthy(now) && acc.has_capacity() {
                            acc.active_ops += 1;
                            state.rr_cursor = (idx + 1) % count;
                            chosen = Some((idx, None));
                            break;
                        }
                    }
                }

                chosen
            };

            if let Some((acc_idx, matched_hot)) = candidate {
                match self.ensure_account_client(acc_idx).await {
                    Ok(client) => {
                        let session_id = matched_hot
                            .as_ref()
                            .map(|h| h.session_id.clone())
                            .unwrap_or_else(|| Uuid::new_v4().to_string());
                        return Ok(VmOperationLease {
                            pool: self.clone(),
                            account_index: acc_idx,
                            client,
                            session_id,
                            matched_hot,
                            saved_as_hot: false,
                            cleaned: false,
                        });
                    }
                    Err((kind, _reason)) => {
                        {
                            let mut state =
                                self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
                            if let Some(acc) = state.accounts.get_mut(acc_idx) {
                                acc.active_ops = acc.active_ops.saturating_sub(1);
                            }
                        }
                        self.fail_account(acc_idx, kind).await;
                        continue;
                    }
                }
            }

            self.inner.notify.notified().await;
        }
    }

    pub(super) async fn ensure_account_client(
        &self,
        account_index: usize,
    ) -> Result<Arc<MuseVmClient>, (UpstreamFailureKind, String)> {
        let cred = {
            let state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
            let acc = state.accounts.get(account_index).ok_or_else(|| {
                (
                    UpstreamFailureKind::Transient,
                    "invalid Muse account index".to_string(),
                )
            })?;
            if let Some(existing) = &acc.vm_client
                && existing.is_connected()
                && acc
                    .credential
                    .wss_url
                    .as_deref()
                    .is_none_or(|u| is_wss_valid(u, WSS_EXPIRY_BUFFER_SECS))
            {
                return Ok(Arc::clone(existing));
            }
            acc.credential.clone()
        };

        if let Some(existing_wss) = cred
            .wss_url
            .as_deref()
            .filter(|u| is_wss_valid(u, WSS_EXPIRY_BUFFER_SECS))
        {
            match connect_and_init_client(&*self.inner.transport, existing_wss).await {
                Ok(client) => {
                    let mut state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some(acc) = state.accounts.get_mut(account_index) {
                        acc.vm_client = Some(Arc::clone(&client));
                    }
                    return Ok(client);
                }
                Err(VmError::AuthInvalid(msg)) if cred.cookies.is_empty() => {
                    return Err((UpstreamFailureKind::AuthInvalid, msg));
                }
                Err(_) if !cred.cookies.is_empty() => {
                    // Fall through to bootstrap a fresh WSS URL from cookies
                }
                Err(err) => {
                    return Err(vm_error_to_pool_failure(err));
                }
            }
        }

        if cred.cookies.is_empty() {
            return Err((
                UpstreamFailureKind::AuthInvalid,
                "Muse WSS URL expired and no session cookie is available for renewal".to_string(),
            ));
        }

        let wss_url = if self.inner.has_custom_transport {
            let vm_name = cred.vm_id.as_deref().unwrap_or(&cred.label);
            format!("wss://hatch.metaaivm.com/v1/noise?vm_id={vm_name}&app_id=hatch-web")
        } else {
            let cookie_header = format_cookie_header(&cred.cookies);
            let bootstrapped =
                match fetch_wss_from_cookie_http(&self.inner.http, &cookie_header, SITE_URL).await {
                    Ok(session) => session,
                    Err((UpstreamFailureKind::Transient, _))
                        if self.inner.fallback_browser.is_some() =>
                    {
                        let engine = self
                            .inner
                            .fallback_browser
                            .as_ref()
                            .expect("fallback_browser checked");
                        let ctx_key = format!("muse-bootstrap-{account_index}");
                        extract_wss_via_chromium(&**engine, &ctx_key, &cred.cookies).await?
                    }
                    Err(err) => return Err(err),
                };

            let mut state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(acc) = state.accounts.get_mut(account_index) {
                acc.credential.wss_url = Some(bootstrapped.wss_url.clone());
                acc.credential.wss_expires = get_wss_expiry_unix_secs(&bootstrapped.wss_url);
                acc.credential.cookies = parse_cookie_header(&bootstrapped.cookie_header);
                if bootstrapped.vm_id.is_some() {
                    acc.credential.vm_id = bootstrapped.vm_id;
                }
            }
            bootstrapped.wss_url
        };

        let client = connect_and_init_client(&*self.inner.transport, &wss_url)
            .await
            .map_err(vm_error_to_pool_failure)?;

        {
            let mut state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(acc) = state.accounts.get_mut(account_index) {
                if acc.credential.wss_url.is_none() {
                    acc.credential.wss_url = Some(wss_url);
                }
                acc.vm_client = Some(Arc::clone(&client));
            }
        }
        Ok(client)
    }

    pub(super) fn release_op_clean(
        &self,
        account_index: usize,
        updated_hot: Option<HotConversation>,
    ) {
        {
            let mut state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(acc) = state.accounts.get_mut(account_index) {
                acc.active_ops = acc.active_ops.saturating_sub(1);
                if let Some(hot) = updated_hot {
                    acc.save_hot_session(hot, Instant::now());
                }
            }
        }
        self.inner.notify.notify_waiters();
    }

    pub(super) async fn fail_account(&self, account_index: usize, kind: UpstreamFailureKind) {
        let stale_client = {
            let mut state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
            let mut client_to_close = None;
            if let Some(acc) = state.accounts.get_mut(account_index) {
                acc.active_ops = acc.active_ops.saturating_sub(1);
                client_to_close = acc.mark_failure(kind, Instant::now());
            }
            client_to_close
        };
        if let Some(client) = stale_client {
            client.close().await;
        }
        self.inner.notify.notify_waiters();
    }
}

async fn connect_and_init_client(
    transport: &dyn MuseVmTransport,
    wss_url: &str,
) -> Result<Arc<MuseVmClient>, VmError> {
    let client = MuseVmClient::connect(transport, wss_url).await?;
    if let Err(err) = client.subscribe().await {
        client.close().await;
        return Err(err);
    }
    client.ensure_isolation_policy().await;
    Ok(client)
}

fn vm_error_to_pool_failure(err: VmError) -> (UpstreamFailureKind, String) {
    match err {
        VmError::AuthInvalid(msg) => (UpstreamFailureKind::AuthInvalid, msg),
        VmError::QuotaExceeded(msg) => (UpstreamFailureKind::QuotaExceeded, msg),
        VmError::Timeout(msg) | VmError::Transport(msg) => (UpstreamFailureKind::Transient, msg),
    }
}
