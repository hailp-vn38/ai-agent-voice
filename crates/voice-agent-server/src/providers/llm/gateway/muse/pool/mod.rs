mod account;
mod lease;
mod types;
mod warmup;

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use tokio::sync::Notify;

use super::cdp::{BrowserEngine, BrowserPage, CdpBrowserEngine, CdpError};
use super::config::MuseAccountCredential;

use account::MuseAccountState;
pub(crate) use lease::TabLease;
pub use types::HOT_BUBBLE_LIMIT;
#[allow(unused_imports)]
pub(crate) use types::{
    AccountHealth, CheckoutMode, ConversationTurn, HotConversation, MusePoolKey, PooledTab,
    UpstreamFailureKind,
};

struct PoolInner {
    accounts: Vec<MuseAccountState>,
    rr_cursor: usize,
}

pub(crate) struct SharedMusePool {
    key: MusePoolKey,
    engine: Arc<dyn BrowserEngine>,
    http_session_warmup: bool,
    http_client: reqwest::Client,
    inner: Arc<StdMutex<PoolInner>>,
    notify: Arc<Notify>,
}

fn spawn_page_close(page: Arc<dyn BrowserPage>) {
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        handle.spawn(async move {
            page.close().await;
        });
    }
}

static GLOBAL_POOLS: OnceLock<StdMutex<HashMap<(MusePoolKey, usize), Arc<SharedMusePool>>>> =
    OnceLock::new();
static GLOBAL_CDP_ENGINES: OnceLock<StdMutex<HashMap<(u16, bool), Arc<dyn BrowserEngine>>>> =
    OnceLock::new();

fn shared_cdp_engine(cdp_port: u16, headless: bool) -> Arc<dyn BrowserEngine> {
    let map_mutex = GLOBAL_CDP_ENGINES.get_or_init(|| StdMutex::new(HashMap::new()));
    let mut map = map_mutex.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(existing) = map.get(&(cdp_port, headless)) {
        return Arc::clone(existing);
    }
    let engine: Arc<dyn BrowserEngine> = Arc::new(CdpBrowserEngine::new(cdp_port, headless));
    map.insert((cdp_port, headless), Arc::clone(&engine));
    engine
}

enum CheckoutPlan {
    Reuse {
        account_idx: usize,
        tab: PooledTab,
        matched_hot: Option<HotConversation>,
    },
    OpenNew {
        account_idx: usize,
    },
}

impl SharedMusePool {
    pub(crate) fn for_gateway_lease(
        cdp_port: u16,
        headless: bool,
        account_concurrency: usize,
        account_key: &str,
        raw_credential: &str,
        custom_engine: Option<Arc<dyn BrowserEngine>>,
    ) -> Arc<Self> {
        let mut hasher = Sha256::new();
        hasher.update(account_key.as_bytes());
        hasher.update(b":");
        hasher.update(raw_credential.as_bytes());
        let credential_fingerprint: [u8; 32] = hasher.finalize().into();

        let key = MusePoolKey {
            cdp_port,
            headless,
            account_concurrency: account_concurrency.clamp(1, 8),
            credential_fingerprint,
        };
        let engine_id = custom_engine
            .as_ref()
            .map(|e| Arc::as_ptr(e) as *const () as usize)
            .unwrap_or(0);

        let map_mutex = GLOBAL_POOLS.get_or_init(|| StdMutex::new(HashMap::new()));
        let mut map = map_mutex.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(existing) = map.get(&(key.clone(), engine_id)) {
            existing.reset_health_for_gateway_lease();
            return Arc::clone(existing);
        }

        let mut accounts = super::config::parse_accounts(raw_credential);
        if accounts.is_empty() && !raw_credential.trim().is_empty() {
            let mut cookies = BTreeMap::new();
            for &name in super::config::REQUIRED_COOKIES {
                cookies.insert(name.to_owned(), raw_credential.trim().to_owned());
            }
            accounts.push(MuseAccountCredential {
                label: account_key.to_owned(),
                cookies,
                cookie_expires: None,
                vm_id: None,
            });
        } else {
            for acc in &mut accounts {
                acc.label = account_key.to_owned();
            }
        }

        let (engine, warmup) = match custom_engine {
            Some(eng) => (eng, false),
            None => (shared_cdp_engine(cdp_port, headless), true),
        };
        let pool = Self::new_with_engine(key.clone(), accounts, engine, warmup);
        map.insert((key, engine_id), Arc::clone(&pool));
        pool
    }

    fn reset_health_for_gateway_lease(&self) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        for acc in &mut guard.accounts {
            acc.health = AccountHealth::Healthy;
        }
    }

    pub fn new_with_engine(
        key: MusePoolKey,
        credentials: Vec<MuseAccountCredential>,
        engine: Arc<dyn BrowserEngine>,
        http_session_warmup: bool,
    ) -> Arc<Self> {
        let accounts = credentials
            .into_iter()
            .map(MuseAccountState::new)
            .collect::<Vec<_>>();
        let inner = Arc::new(StdMutex::new(PoolInner {
            accounts,
            rr_cursor: 0,
        }));
        let notify = Arc::new(Notify::new());

        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .unwrap_or_default();

        Arc::new(Self {
            key,
            engine,
            http_session_warmup,
            http_client,
            inner,
            notify,
        })
    }

    pub async fn checkout(&self, mode: CheckoutMode) -> Result<TabLease, CdpError> {
        loop {
            let mut stale_to_close: Vec<Arc<dyn BrowserPage>> = Vec::new();
            let selection = {
                let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
                let now = Instant::now();
                let total = guard.accounts.len();
                if total == 0 {
                    return Err(CdpError::BrowserUnavailable(
                        "no valid Muse account credentials configured".to_string(),
                    ));
                }

                for acc in &mut guard.accounts {
                    acc.refresh_health(now);
                    let mut keep = Vec::with_capacity(acc.idle_tabs.len());
                    for tab in acc.idle_tabs.drain(..) {
                        if tab.stale {
                            acc.active_tabs = acc.active_tabs.saturating_sub(1);
                            stale_to_close.push(tab.page);
                        } else {
                            keep.push(tab);
                        }
                    }
                    acc.idle_tabs = keep;
                }

                if !guard
                    .accounts
                    .iter()
                    .any(|a| !matches!(a.health, AccountHealth::Invalid))
                {
                    return Err(CdpError::Protocol(
                        "all Muse accounts have invalid authentication cookies".to_string(),
                    ));
                }

                // 1. If Continuation mode, look for a hot tab matching the conversation prefix.
                let mut hot_found = None;
                if let CheckoutMode::Continuation {
                    ref prior_turns,
                    image_count,
                } = mode
                {
                    if !prior_turns.is_empty() && (prior_turns.len() * 2 + 2) <= HOT_BUBBLE_LIMIT {
                        'outer: for (acc_idx, acc) in guard.accounts.iter_mut().enumerate() {
                            if !acc.is_available(now) {
                                continue;
                            }
                            if let Some(pos) = acc.idle_tabs.iter().position(|t| {
                                t.hot.as_ref().is_some_and(|h| {
                                    !h.turns.is_empty()
                                        && h.turns.len() <= prior_turns.len()
                                        && prior_turns.starts_with(&h.turns)
                                        && h.image_count == image_count
                                })
                            }) {
                                let tab = acc.idle_tabs.remove(pos);
                                let matched_hot = tab.hot.clone();
                                hot_found = Some((acc_idx, tab, matched_hot));
                                break 'outer;
                            }
                        }
                    }
                }

                if let Some((acc_idx, tab, matched_hot)) = hot_found {
                    Some(CheckoutPlan::Reuse {
                        account_idx: acc_idx,
                        tab,
                        matched_hot,
                    })
                } else {
                    // 2. Round-robin across available accounts.
                    let start = guard.rr_cursor % total;
                    let mut chosen = None;
                    for offset in 0..total {
                        let idx = (start + offset) % total;
                        let acc = &mut guard.accounts[idx];
                        if !acc.is_available(now) {
                            continue;
                        }
                        if let Some(pos) = acc.idle_tabs.iter().position(|t| t.hot.is_none()) {
                            let tab = acc.idle_tabs.remove(pos);
                            guard.rr_cursor = idx + 1;
                            chosen = Some(CheckoutPlan::Reuse {
                                account_idx: idx,
                                tab,
                                matched_hot: None,
                            });
                            break;
                        }
                        if acc.active_tabs < self.key.account_concurrency {
                            acc.active_tabs += 1;
                            guard.rr_cursor = idx + 1;
                            chosen = Some(CheckoutPlan::OpenNew { account_idx: idx });
                            break;
                        }
                        if let Some(mut tab) = acc.idle_tabs.pop() {
                            tab.hot = None;
                            guard.rr_cursor = idx + 1;
                            chosen = Some(CheckoutPlan::Reuse {
                                account_idx: idx,
                                tab,
                                matched_hot: None,
                            });
                            break;
                        }
                    }
                    chosen
                }
            };

            for page in stale_to_close {
                spawn_page_close(page);
            }

            match selection {
                Some(CheckoutPlan::Reuse {
                    account_idx,
                    tab,
                    matched_hot,
                }) => {
                    return Ok(TabLease {
                        account_idx,
                        tab: Some(tab),
                        matched_hot,
                        clean_return: false,
                        inner: Arc::clone(&self.inner),
                        notify: Arc::clone(&self.notify),
                    });
                }
                Some(CheckoutPlan::OpenNew { account_idx }) => {
                    if let Err(kind) = self.ensure_account_warm(account_idx).await {
                        {
                            let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
                            if let Some(acc) = guard.accounts.get_mut(account_idx) {
                                acc.active_tabs = acc.active_tabs.saturating_sub(1);
                            }
                        }
                        self.apply_account_failure(account_idx, kind);
                        continue;
                    }

                    let (context_key, cookie_params) = {
                        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
                        let acc = &guard.accounts[account_idx];
                        (
                            acc.context_key(&self.key.credential_fingerprint),
                            acc.cookie_params(),
                        )
                    };

                    match self.engine.open_page(&context_key, &cookie_params).await {
                        Ok(page) => {
                            return Ok(TabLease {
                                account_idx,
                                tab: Some(PooledTab {
                                    page,
                                    hot: None,
                                    stale: false,
                                }),
                                matched_hot: None,
                                clean_return: false,
                                inner: Arc::clone(&self.inner),
                                notify: Arc::clone(&self.notify),
                            });
                        }
                        Err(err) => {
                            {
                                let mut guard =
                                    self.inner.lock().unwrap_or_else(|e| e.into_inner());
                                if let Some(acc) = guard.accounts.get_mut(account_idx) {
                                    acc.active_tabs = acc.active_tabs.saturating_sub(1);
                                }
                            }
                            self.notify.notify_waiters();
                            return Err(err);
                        }
                    }
                }
                None => {
                    let all_cooling = {
                        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
                        let now = Instant::now();
                        !guard.accounts.iter().any(|a| a.is_available(now))
                    };
                    if all_cooling {
                        return Err(CdpError::Protocol(
                            "all Muse accounts are currently in quota/transient cooldown"
                                .to_string(),
                        ));
                    }
                    self.notify.notified().await;
                }
            }
        }
    }
}

impl Drop for SharedMusePool {
    fn drop(&mut self) {
        let engine = Arc::clone(&self.engine);
        let context_keys = {
            let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            guard
                .accounts
                .iter()
                .map(|acc| acc.context_key(&self.key.credential_fingerprint))
                .collect::<Vec<_>>()
        };
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                for key in context_keys {
                    engine.dispose_context(&key).await;
                }
            });
        }
    }
}
