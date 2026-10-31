mod account;
mod connect;
mod job_sessions;
mod lease;
mod types;
mod warmup;

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Instant;

use tokio::sync::Notify;
use uuid::Uuid;

use self::account::{AccountStatus, MuseAccountState};
pub(crate) use self::job_sessions::{JobEntry, JobSession, job_entry};
pub(crate) use self::lease::VmOperationLease;
pub(crate) use self::types::{ConversationTurn, HotConversation, UpstreamFailureKind};
use super::cdp::BrowserEngine;
use super::client::{MuseVmTransport, WsNoiseVmTransport};
use super::config::{MuseAccountCredential, parse_accounts};

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
        let prefix = format!("gw:{account_id}:{transport_id}:");
        let cache_key = format!("{prefix}{credential_json}");
        let mut map = GLOBAL_POOLS.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(existing) = map.get(&cache_key) {
            return Arc::clone(existing);
        }
        if let Some(target_cred) = credentials.first()
            && let Some(existing_pool) = map
                .iter()
                .find(|(k, _)| k.starts_with(&prefix))
                .map(|(_, p)| Arc::clone(p))
        {
            let can_reuse = {
                let state = existing_pool
                    .inner
                    .state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                state.accounts.first().is_some_and(|acc| {
                    matches!(acc.status, AccountStatus::Healthy)
                        && acc.credential.wss_url == target_cred.wss_url
                        && acc.credential.cookies == target_cred.cookies
                })
            };
            if can_reuse {
                map.retain(|k, _| !k.starts_with(&prefix));
                map.insert(cache_key, Arc::clone(&existing_pool));
                return existing_pool;
            }
        }
        map.retain(|k, _| !k.starts_with(&prefix));
        let has_custom = custom_transport.is_some();
        let transport: Arc<dyn MuseVmTransport> =
            custom_transport.unwrap_or_else(|| Arc::new(WsNoiseVmTransport));
        #[cfg(not(test))]
        let fallback_browser: Option<Arc<dyn BrowserEngine>> = if has_custom {
            None
        } else {
            Some(Arc::new(super::cdp::CdpBrowserEngine::new(19222, true)))
        };
        #[cfg(test)]
        let fallback_browser: Option<Arc<dyn BrowserEngine>> = None;
        let pool = Arc::new(Self::new_with_options(
            http.clone(),
            transport,
            has_custom,
            fallback_browser,
            credentials,
        ));
        {
            let mut state = pool.inner.state.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(acc) = state.accounts.first_mut() {
                acc.gateway_account_key = Some(account_id.to_string());
            }
        }
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
        let accounts: Vec<MuseAccountState> =
            credentials.into_iter().map(MuseAccountState::new).collect();
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
        self.acquire_internal(prior_turns, true, None).await
    }

    pub async fn acquire_fresh(&self) -> Result<VmOperationLease, (UpstreamFailureKind, String)> {
        self.acquire_internal(&[], false, None).await
    }

    /// Acquires a lease that talks on `session_id` (a retained job session) instead of a new one.
    pub async fn acquire_existing_session(
        &self,
        session_id: String,
    ) -> Result<VmOperationLease, (UpstreamFailureKind, String)> {
        self.acquire_internal(&[], false, Some(session_id)).await
    }

    /// Gateway account key of this pool's (single) account, used to validate session reuse.
    pub fn gateway_account_key(&self) -> Option<String> {
        let state = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        state
            .accounts
            .first()
            .and_then(|acc| acc.gateway_account_key.clone())
    }

    async fn acquire_internal(
        &self,
        prior_turns: &[ConversationTurn],
        prefer_hot: bool,
        forced_session_id: Option<String>,
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
                            .or_else(|| forced_session_id.clone())
                            .unwrap_or_else(|| Uuid::new_v4().to_string());
                        let account_key = self
                            .inner
                            .state
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .accounts
                            .get(acc_idx)
                            .and_then(|acc| acc.gateway_account_key.clone());
                        return Ok(VmOperationLease {
                            pool: self.clone(),
                            account_index: acc_idx,
                            client,
                            session_id,
                            matched_hot,
                            saved_as_hot: false,
                            cleaned: false,
                            account_key,
                            retain_session: false,
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
