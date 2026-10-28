use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex as StdMutex, OnceLock, Weak};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};
use tokio::sync::{mpsc, Notify};
use uuid::Uuid;

use super::cdp::{BrowserEngine, BrowserPage, CdpBrowserEngine, CdpError, CookieParam};
use super::config::{
    MuseAccountCredential, MuseConfig, COOKIE_DOMAIN, NEW_THREAD_URL, SESSION_API_URL, SITE_URL,
    VM_WAKE_API_URL,
};

pub const HOT_BUBBLE_LIMIT: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct MusePoolKey {
    pub cdp_port: u16,
    pub headless: bool,
    pub account_concurrency: usize,
    /// SHA-256 digest of `api_key.expose()` ensuring secret rotation creates isolated contexts.
    pub credential_fingerprint: [u8; 32],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AccountHealth {
    Healthy,
    Cooldown(Instant),
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum UpstreamFailureKind {
    AuthInvalid,
    QuotaExceeded,
    Transient,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ConversationTurn {
    pub user: String,
    pub assistant: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct HotConversation {
    pub hint: String,
    pub turns: Vec<ConversationTurn>,
    pub image_count: usize,
    pub thread_url: String,
}

pub(super) struct PooledTab {
    pub page: Arc<dyn BrowserPage>,
    pub hot: Option<HotConversation>,
    pub stale: bool,
}

#[derive(Debug, Clone)]
pub(super) enum CheckoutMode {
    Stateless,
    Continuation {
        prior_turns: Vec<ConversationTurn>,
        image_count: usize,
    },
}

struct MuseAccountState {
    credential: MuseAccountCredential,
    in_memory_cookies: BTreeMap<String, String>,
    cookie_expires: Option<u64>,
    vm_id: Option<String>,
    warmed: bool,
    generation: u64,
    health: AccountHealth,
    active_tabs: usize,
    idle_tabs: Vec<PooledTab>,
}

impl MuseAccountState {
    fn new(credential: MuseAccountCredential) -> Self {
        let in_memory_cookies = credential.cookies.clone();
        let cookie_expires = credential.cookie_expires;
        let vm_id = credential.vm_id.clone();
        Self {
            credential,
            in_memory_cookies,
            cookie_expires,
            vm_id,
            warmed: false,
            generation: 0,
            health: AccountHealth::Healthy,
            active_tabs: 0,
            idle_tabs: Vec::new(),
        }
    }

    fn is_available(&self, now: Instant) -> bool {
        match self.health {
            AccountHealth::Healthy => true,
            AccountHealth::Cooldown(until) => now >= until,
            AccountHealth::Invalid => false,
        }
    }

    fn refresh_health(&mut self, now: Instant) {
        if let AccountHealth::Cooldown(until) = self.health {
            if now >= until {
                self.health = AccountHealth::Healthy;
            }
        }
    }

    fn context_key(&self, fingerprint: &[u8; 32]) -> String {
        let prefix = hex_prefix(&fingerprint[..8]);
        format!("{prefix}:{}:{}", self.credential.label, self.generation)
    }

    fn cookie_params(&self) -> Vec<CookieParam> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64();
        let floor = now + 3600.0;
        let expires = match self.cookie_expires.map(|e| e as f64) {
            Some(exp) if exp > floor => exp,
            _ => now + 7.0 * 86400.0,
        };
        self.in_memory_cookies
            .iter()
            .filter(|(_, v)| !v.is_empty())
            .map(|(k, v)| CookieParam {
                name: k.clone(),
                value: v.clone(),
                domain: COOKIE_DOMAIN.to_string(),
                path: "/".to_string(),
                secure: true,
                expires,
            })
            .collect()
    }
}

fn hex_prefix(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(&mut out, "{b:02x}");
    }
    out
}

struct PoolInner {
    accounts: Vec<MuseAccountState>,
    rr_cursor: usize,
}

pub(super) struct SharedMusePool {
    key: MusePoolKey,
    engine: Arc<dyn BrowserEngine>,
    http_session_warmup: bool,
    http_client: reqwest::Client,
    inner: Arc<StdMutex<PoolInner>>,
    notify: Arc<Notify>,
    dispose_tx: mpsc::UnboundedSender<Arc<dyn BrowserPage>>,
}

static GLOBAL_POOLS: OnceLock<StdMutex<HashMap<MusePoolKey, Weak<SharedMusePool>>>> =
    OnceLock::new();

impl SharedMusePool {
    pub fn get_or_create(config: &MuseConfig) -> Arc<Self> {
        let mut hasher = Sha256::new();
        hasher.update(config.api_key.expose().as_bytes());
        let credential_fingerprint: [u8; 32] = hasher.finalize().into();

        let key = MusePoolKey {
            cdp_port: config.cdp_port,
            headless: config.headless,
            account_concurrency: config.account_concurrency.clamp(1, 8),
            credential_fingerprint,
        };

        let map_mutex = GLOBAL_POOLS.get_or_init(|| StdMutex::new(HashMap::new()));
        let mut map = map_mutex.lock().unwrap_or_else(|e| e.into_inner());
        map.retain(|_, weak| weak.strong_count() > 0);
        if let Some(existing) = map.get(&key).and_then(Weak::upgrade) {
            return existing;
        }

        let engine: Arc<dyn BrowserEngine> =
            Arc::new(CdpBrowserEngine::new(config.cdp_port, config.headless));
        let pool = Self::new_with_engine(key.clone(), config.parse_accounts(), engine, true);
        map.insert(key, Arc::downgrade(&pool));
        pool
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
        let (dispose_tx, mut dispose_rx) = mpsc::unbounded_channel::<Arc<dyn BrowserPage>>();

        tokio::spawn(async move {
            while let Some(page) = dispose_rx.recv().await {
                page.close().await;
            }
        });

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
            dispose_tx,
        })
    }

    pub fn num_accounts(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .accounts
            .len()
    }

    fn apply_account_failure(&self, account_idx: usize, kind: UpstreamFailureKind) {
        let mut stale_pages = Vec::new();
        {
            let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(acc) = guard.accounts.get_mut(account_idx) {
                acc.health = match kind {
                    UpstreamFailureKind::AuthInvalid => AccountHealth::Invalid,
                    UpstreamFailureKind::QuotaExceeded => {
                        AccountHealth::Cooldown(Instant::now() + Duration::from_secs(3600))
                    }
                    UpstreamFailureKind::Transient => {
                        AccountHealth::Cooldown(Instant::now() + Duration::from_secs(30))
                    }
                };
                if matches!(kind, UpstreamFailureKind::AuthInvalid) {
                    for idle in acc.idle_tabs.drain(..) {
                        acc.active_tabs = acc.active_tabs.saturating_sub(1);
                        stale_pages.push(idle.page);
                    }
                }
            }
        }
        for page in stale_pages {
            let _ = self.dispose_tx.send(page);
        }
        self.notify.notify_waiters();
    }

    async fn ensure_account_warm(&self, account_idx: usize) -> Result<(), UpstreamFailureKind> {
        if !self.http_session_warmup {
            let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(acc) = guard.accounts.get_mut(account_idx) {
                acc.warmed = true;
            }
            return Ok(());
        }

        let (already_warmed, cookies, vm_id) = {
            let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            let Some(acc) = guard.accounts.get(account_idx) else {
                return Err(UpstreamFailureKind::AuthInvalid);
            };
            (acc.warmed, acc.in_memory_cookies.clone(), acc.vm_id.clone())
        };
        if already_warmed {
            return Ok(());
        }

        let cookie_header = cookies
            .iter()
            .filter(|(_, v)| !v.is_empty())
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("; ");

        let resp = self
            .http_client
            .get(SESSION_API_URL)
            .header("Origin", SITE_URL)
            .header("Referer", NEW_THREAD_URL)
            .header("Accept", "application/json")
            .header("Cookie", &cookie_header)
            .send()
            .await;

        let mut rotated_cookies = cookies;
        let mut discovered_vm_id = vm_id;

        if let Ok(resp) = resp {
            let status = resp.status().as_u16();
            if status == 401 || status == 403 {
                self.apply_account_failure(account_idx, UpstreamFailureKind::AuthInvalid);
                return Err(UpstreamFailureKind::AuthInvalid);
            }
            for header_val in resp.headers().get_all(reqwest::header::SET_COOKIE) {
                if let Ok(raw) = header_val.to_str() {
                    if let Some(first_part) = raw.split(';').next() {
                        if let Some((k, v)) = first_part.split_once('=') {
                            let k = k.trim();
                            let v = v.trim();
                            if !k.is_empty() && !v.is_empty() {
                                rotated_cookies.insert(k.to_string(), v.to_string());
                            }
                        }
                    }
                }
            }
            if let Ok(body) = resp.json::<serde_json::Value>().await {
                if discovered_vm_id.is_none() {
                    discovered_vm_id = body
                        .get("vm_id")
                        .and_then(serde_json::Value::as_str)
                        .map(ToString::to_string);
                }
            }
        }

        if let Some(vm) = &discovered_vm_id {
            let updated_cookie_header = rotated_cookies
                .iter()
                .filter(|(_, v)| !v.is_empty())
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("; ");
            let _ = self
                .http_client
                .post(VM_WAKE_API_URL)
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

        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(acc) = guard.accounts.get_mut(account_idx) {
            if acc.in_memory_cookies != rotated_cookies {
                acc.in_memory_cookies = rotated_cookies;
                acc.cookie_expires = Some(
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs()
                        + 7 * 86400,
                );
                for tab in &mut acc.idle_tabs {
                    tab.stale = true;
                }
            }
            acc.vm_id = discovered_vm_id;
            acc.warmed = true;
        }
        Ok(())
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
                let _ = self.dispose_tx.send(page);
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
                        dispose_tx: self.dispose_tx.clone(),
                    });
                }
                Some(CheckoutPlan::OpenNew { account_idx }) => {
                    if let Err(kind) = self.ensure_account_warm(account_idx).await {
                        {
                            let mut guard =
                                self.inner.lock().unwrap_or_else(|e| e.into_inner());
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
                                dispose_tx: self.dispose_tx.clone(),
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

pub(super) struct TabLease {
    account_idx: usize,
    tab: Option<PooledTab>,
    matched_hot: Option<HotConversation>,
    clean_return: bool,
    inner: Arc<StdMutex<PoolInner>>,
    notify: Arc<Notify>,
    dispose_tx: mpsc::UnboundedSender<Arc<dyn BrowserPage>>,
}

impl TabLease {
    pub fn page(&self) -> &Arc<dyn BrowserPage> {
        &self
            .tab
            .as_ref()
            .expect("TabLease always holds a tab while borrowed")
            .page
    }

    pub fn matched_hot(&self) -> Option<&HotConversation> {
        self.matched_hot.as_ref()
    }

    pub fn finish_clean(&mut self, hot: Option<HotConversation>) {
        if let Some(tab) = &mut self.tab {
            tab.hot = hot;
        }
        self.clean_return = true;
    }

    pub fn fail_account(&self, kind: UpstreamFailureKind) {
        let mut stale_pages = Vec::new();
        {
            let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(acc) = guard.accounts.get_mut(self.account_idx) {
                acc.health = match kind {
                    UpstreamFailureKind::AuthInvalid => AccountHealth::Invalid,
                    UpstreamFailureKind::QuotaExceeded => {
                        AccountHealth::Cooldown(Instant::now() + Duration::from_secs(3600))
                    }
                    UpstreamFailureKind::Transient => {
                        AccountHealth::Cooldown(Instant::now() + Duration::from_secs(30))
                    }
                };
                if matches!(kind, UpstreamFailureKind::AuthInvalid) {
                    for idle in acc.idle_tabs.drain(..) {
                        acc.active_tabs = acc.active_tabs.saturating_sub(1);
                        stale_pages.push(idle.page);
                    }
                }
            }
        }
        for page in stale_pages {
            let _ = self.dispose_tx.send(page);
        }
        self.notify.notify_waiters();
    }
}

impl Drop for TabLease {
    fn drop(&mut self) {
        let Some(tab) = self.tab.take() else {
            return;
        };
        let page_to_close = {
            let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(acc) = guard.accounts.get_mut(self.account_idx) {
                if self.clean_return
                    && !tab.stale
                    && !matches!(acc.health, AccountHealth::Invalid)
                {
                    acc.idle_tabs.push(tab);
                    None
                } else {
                    acc.active_tabs = acc.active_tabs.saturating_sub(1);
                    Some(tab.page)
                }
            } else {
                Some(tab.page)
            }
        };
        self.notify.notify_waiters();
        if let Some(page) = page_to_close {
            let _ = self.dispose_tx.send(page);
        }
    }
}
