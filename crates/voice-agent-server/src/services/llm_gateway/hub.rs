use std::{
    collections::{BTreeMap, HashMap},
    sync::{
        Arc, Mutex, OnceLock, RwLock,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use serde_json::Value;
use sqlx::SqlitePool;

use crate::{
    config::SecretString,
    database::{
        credentials,
        secrets::{EnvSecretResolver, SecretResolver},
        unix_seconds,
    },
};

use super::{
    oauth_flow::{
        OAUTH_SESSION_TTL, OAuthLoginSession, OAuthSessionFlowState, generate_pkce_pair,
        generate_state_token, parse_oauth_callback_submission,
    },
    types::{
        GatewayAccountQuota, GatewayAccountStatus, GatewayAccountView, GatewayBinding,
        GatewayBoundProviderRef, GatewayCallOutcome, GatewayError, GatewayLease, GatewayVendor,
        GatewayVendorInfo, OAuthCallbackSubmission, OAuthLoginChallenge, OAuthLoginMode,
        OAuthPollStatus, OAuthTokenBundle, StartOAuthLoginRequest, UpdateGatewayAccountPatch,
        valid_account_key,
    },
    vendors::{
        DevicePollOutcome, ExchangedOAuthAccount, OAuthVendorDriver, default_vendor_drivers,
    },
};

const DEFAULT_COOLDOWN_SECS: u64 = 60;
const MIN_COOLDOWN_SECS: u64 = 15;
const MAX_COOLDOWN_SECS: u64 = 600;
const QUOTA_CACHE_TTL_SECS: i64 = 60;

#[derive(Debug, Clone)]
pub(crate) struct LoadedGatewayAccount {
    pub account_key: String,
    pub vendor: GatewayVendor,
    pub label: String,
    pub account_email: Option<String>,
    pub credential_json: String,
    pub bundle: Option<OAuthTokenBundle>,
    pub extra_headers: Vec<(String, String)>,
    pub expires_at: Option<i64>,
    pub enabled: bool,
    pub priority: i64,
    pub status: GatewayAccountStatus,
    pub last_error: Option<String>,
    pub last_refreshed_at: Option<i64>,
    pub last_used_at: Option<i64>,
    pub cooldown_until: Option<Instant>,
}

impl LoadedGatewayAccount {
    pub fn cooldown_remaining_secs(&self, now: Instant) -> u64 {
        self.cooldown_until
            .and_then(|until| until.checked_duration_since(now))
            .map(|d| d.as_secs().max(1))
            .unwrap_or(0)
    }

    pub fn is_ready_for_lease(&self, now: Instant) -> bool {
        self.enabled
            && self.status == GatewayAccountStatus::Active
            && self.bundle.is_some()
            && self.cooldown_remaining_secs(now) == 0
    }
}

#[derive(Debug, Default, Clone)]
pub(crate) struct GatewayRuntimeSnapshot {
    pub accounts: BTreeMap<String, LoadedGatewayAccount>,
    pub ordered_keys_by_vendor: BTreeMap<GatewayVendor, Vec<String>>,
}

impl GatewayRuntimeSnapshot {
    fn rebuild_vendor_index(&mut self) {
        self.ordered_keys_by_vendor.clear();
        for vendor in GatewayVendor::ALL {
            let mut list: Vec<&LoadedGatewayAccount> = self
                .accounts
                .values()
                .filter(|acc| acc.vendor == vendor)
                .collect();
            list.sort_by(|a, b| {
                a.priority
                    .cmp(&b.priority)
                    .then_with(|| a.account_key.cmp(&b.account_key))
            });
            self.ordered_keys_by_vendor.insert(
                vendor,
                list.into_iter().map(|a| a.account_key.clone()).collect(),
            );
        }
    }
}

pub struct LlmGatewayHub {
    pool: Option<SqlitePool>,
    secret_resolver: Arc<dyn SecretResolver>,
    drivers: BTreeMap<GatewayVendor, Arc<dyn OAuthVendorDriver>>,
    snapshot: RwLock<GatewayRuntimeSnapshot>,
    quotas: RwLock<HashMap<String, GatewayAccountQuota>>,
    sessions: Mutex<HashMap<String, OAuthLoginSession>>,
    refresh_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    pool_cursors: BTreeMap<GatewayVendor, AtomicUsize>,
}

impl std::fmt::Debug for LlmGatewayHub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LlmGatewayHub")
            .field("has_pool", &self.pool.is_some())
            .finish()
    }
}

impl LlmGatewayHub {
    pub fn new(pool: Option<SqlitePool>, secret_resolver: Arc<dyn SecretResolver>) -> Self {
        Self::with_drivers(pool, secret_resolver, default_vendor_drivers())
    }

    pub fn with_drivers(
        pool: Option<SqlitePool>,
        secret_resolver: Arc<dyn SecretResolver>,
        drivers: BTreeMap<GatewayVendor, Arc<dyn OAuthVendorDriver>>,
    ) -> Self {
        let mut pool_cursors = BTreeMap::new();
        for vendor in GatewayVendor::ALL {
            pool_cursors.insert(vendor, AtomicUsize::new(0));
        }
        Self {
            pool,
            secret_resolver,
            drivers,
            snapshot: RwLock::new(GatewayRuntimeSnapshot::default()),
            quotas: RwLock::new(HashMap::new()),
            sessions: Mutex::new(HashMap::new()),
            refresh_locks: Mutex::new(HashMap::new()),
            pool_cursors,
        }
    }

    fn driver_for(
        &self,
        vendor: GatewayVendor,
    ) -> Result<&Arc<dyn OAuthVendorDriver>, GatewayError> {
        self.drivers
            .get(&vendor)
            .ok_or_else(|| GatewayError::OAuthExchangeFailed("driver_missing".to_owned()))
    }

    fn refresh_lock_for(&self, account_key: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut guard = self
            .refresh_locks
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        guard
            .entry(account_key.to_owned())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    }

    /// Reloads the in-RAM `GatewayRuntimeSnapshot` from SQLite `llm_gateway_accounts`,
    /// decrypting `credential_json` out-of-band so hot-path `acquire_lease` performs zero SQL/decryption.
    pub async fn reload_snapshot(&self) -> Result<(), GatewayError> {
        let Some(pool) = &self.pool else {
            return Ok(());
        };

        type RowTuple = (
            String,
            String,
            String,
            Option<String>,
            String,
            Option<i64>,
            i64,
            i64,
            String,
            Option<String>,
            Option<i64>,
            Option<i64>,
        );

        let rows: Vec<RowTuple> = sqlx::query_as(
            "SELECT account_key, vendor, label, account_email, credential_json, \
                    expires_at, enabled, priority, status, last_error, last_refreshed_at, last_used_at \
             FROM llm_gateway_accounts \
             ORDER BY priority ASC, id ASC",
        )
        .fetch_all(pool)
        .await
        .map_err(|_| GatewayError::DatabaseUnavailable)?;

        let existing_cooldowns: HashMap<String, Option<Instant>> = {
            let guard = self.snapshot.read().unwrap_or_else(|p| p.into_inner());
            guard
                .accounts
                .iter()
                .map(|(k, v)| (k.clone(), v.cooldown_until))
                .collect()
        };

        let mut next_snapshot = GatewayRuntimeSnapshot::default();
        for (
            account_key,
            vendor_str,
            label,
            account_email,
            credential_json,
            expires_at,
            enabled_int,
            priority,
            status_str,
            last_error,
            last_refreshed_at,
            last_used_at,
        ) in rows
        {
            let Some(vendor) = GatewayVendor::parse(&vendor_str) else {
                continue;
            };
            let bundle = OAuthTokenBundle::open(
                self.secret_resolver.as_ref(),
                &account_key,
                &credential_json,
            )
            .ok();
            let extra_headers = match (&bundle, self.drivers.get(&vendor)) {
                (Some(b), Some(drv)) => drv.extra_headers_for(b),
                _ => Vec::new(),
            };
            let mut status = GatewayAccountStatus::parse(&status_str);
            if bundle.is_none() && status == GatewayAccountStatus::Active {
                status = GatewayAccountStatus::ReauthRequired;
            }
            let cooldown_until = existing_cooldowns.get(&account_key).copied().flatten();
            next_snapshot.accounts.insert(
                account_key.clone(),
                LoadedGatewayAccount {
                    account_key,
                    vendor,
                    label,
                    account_email,
                    credential_json,
                    bundle,
                    extra_headers,
                    expires_at,
                    enabled: enabled_int != 0,
                    priority,
                    status,
                    last_error,
                    last_refreshed_at,
                    last_used_at,
                    cooldown_until,
                },
            );
        }
        next_snapshot.rebuild_vendor_index();

        let mut guard = self.snapshot.write().unwrap_or_else(|p| p.into_inner());
        *guard = next_snapshot;
        Ok(())
    }

    /// Resolves the canonical default upstream base URL for a `GatewayBinding` by inspecting
    /// the account's `GatewayVendor` in the in-RAM snapshot rather than guessing from string prefixes.
    pub fn default_base_url_for_binding(&self, binding: &GatewayBinding) -> &'static str {
        match binding {
            GatewayBinding::Pool(vendor) => vendor.default_base_url(),
            GatewayBinding::Account(account_key) => {
                let guard = self.snapshot.read().unwrap_or_else(|p| p.into_inner());
                guard
                    .accounts
                    .get(account_key)
                    .map(|acc| acc.vendor.default_base_url())
                    .unwrap_or_else(|| GatewayVendor::OpenaiCodex.default_base_url())
            }
        }
    }

    /// Hot-path synchronous (`0ms`) lease acquisition from the in-RAM `GatewayRuntimeSnapshot`.
    /// Never queries SQLite and never invokes `SecretResolver::resolve` during a voice or task request.
    pub fn acquire_lease(&self, binding: &GatewayBinding) -> Result<GatewayLease, GatewayError> {
        let now = Instant::now();
        let guard = self.snapshot.read().unwrap_or_else(|p| p.into_inner());

        match binding {
            GatewayBinding::Account(account_key) => {
                let acc = guard
                    .accounts
                    .get(account_key)
                    .ok_or(GatewayError::AccountNotFound)?;
                if !acc.enabled || acc.status == GatewayAccountStatus::Disabled {
                    return Err(GatewayError::AccountDisabled);
                }
                if acc.status == GatewayAccountStatus::ReauthRequired {
                    return Err(GatewayError::AccountReauthRequired);
                }
                if acc.cooldown_remaining_secs(now) > 0 {
                    return Err(GatewayError::AccountCoolingDown);
                }
                let bundle = acc
                    .bundle
                    .as_ref()
                    .ok_or(GatewayError::AccountReauthRequired)?;
                Ok(GatewayLease {
                    account_key: acc.account_key.clone(),
                    vendor: acc.vendor,
                    access_token: SecretString::new(bundle.access_token.clone()),
                    base_url: acc.vendor.default_base_url().to_owned(),
                    extra_headers: acc.extra_headers.clone(),
                })
            }
            GatewayBinding::Pool(vendor) => {
                let keys = guard
                    .ordered_keys_by_vendor
                    .get(vendor)
                    .ok_or(GatewayError::PoolExhausted)?;
                let ready: Vec<&LoadedGatewayAccount> = keys
                    .iter()
                    .filter_map(|k| guard.accounts.get(k))
                    .filter(|acc| acc.is_ready_for_lease(now))
                    .collect();
                if ready.is_empty() {
                    return Err(GatewayError::PoolExhausted);
                }
                let quotas_guard = self.quotas.read().unwrap_or_else(|p| p.into_inner());
                let quota_available: Vec<&LoadedGatewayAccount> = ready
                    .iter()
                    .copied()
                    .filter(|acc| {
                        quotas_guard
                            .get(&acc.account_key)
                            .is_none_or(|q| !q.limit_reached)
                    })
                    .collect();
                let candidates = if quota_available.is_empty() {
                    &ready
                } else {
                    &quota_available
                };
                let cursor = self
                    .pool_cursors
                    .get(vendor)
                    .map(|c| c.fetch_add(1, Ordering::Relaxed))
                    .unwrap_or(0);
                let chosen = candidates[cursor % candidates.len()];
                let bundle = chosen
                    .bundle
                    .as_ref()
                    .ok_or(GatewayError::PoolExhausted)?;
                Ok(GatewayLease {
                    account_key: chosen.account_key.clone(),
                    vendor: chosen.vendor,
                    access_token: SecretString::new(bundle.access_token.clone()),
                    base_url: chosen.vendor.default_base_url().to_owned(),
                    extra_headers: chosen.extra_headers.clone(),
                })
            }
        }
    }

    /// Updates in-RAM cooldown or reauth state immediately after an upstream LLM call completes.
    pub fn report_outcome(&self, account_key: &str, outcome: GatewayCallOutcome) {
        let now = Instant::now();
        let now_epoch = unix_seconds().unwrap_or(0);
        let mut guard = self.snapshot.write().unwrap_or_else(|p| p.into_inner());
        let Some(acc) = guard.accounts.get_mut(account_key) else {
            return;
        };
        match outcome {
            GatewayCallOutcome::Success => {
                acc.cooldown_until = None;
                acc.last_used_at = Some(now_epoch);
                acc.last_error = None;
            }
            GatewayCallOutcome::RateLimited { retry_after_secs } => {
                let secs = retry_after_secs
                    .unwrap_or(DEFAULT_COOLDOWN_SECS)
                    .clamp(MIN_COOLDOWN_SECS, MAX_COOLDOWN_SECS);
                acc.cooldown_until = Some(now + Duration::from_secs(secs));
                acc.last_error = Some(format!("rate_limited_cooldown_{secs}s"));
            }
            GatewayCallOutcome::Unauthorized => {
                acc.status = GatewayAccountStatus::ReauthRequired;
                acc.last_error = Some("upstream_unauthorized".to_owned());
            }
        }
    }

    /// Out-of-band proactive renewal for accounts whose `expires_at` is within `margin_secs`.
    pub async fn proactive_refresh_expiring(
        &self,
        margin_secs: i64,
    ) -> Result<usize, GatewayError> {
        let now_epoch = unix_seconds().unwrap_or(0);
        let threshold = now_epoch + margin_secs.max(0);
        let due_keys: Vec<String> = {
            let guard = self.snapshot.read().unwrap_or_else(|p| p.into_inner());
            guard
                .accounts
                .values()
                .filter(|acc| {
                    acc.enabled
                        && acc.status == GatewayAccountStatus::Active
                        && acc.expires_at.is_some_and(|exp| exp <= threshold)
                })
                .map(|acc| acc.account_key.clone())
                .collect()
        };

        let mut refreshed = 0usize;
        for key in due_keys {
            if self.refresh_account_internal(&key, Some(threshold)).await.is_ok() {
                refreshed += 1;
            }
        }
        Ok(refreshed)
    }

    pub fn spawn_proactive_refresh_loop(
        self: &Arc<Self>,
        interval: Duration,
        margin_secs: i64,
    ) -> tokio::task::JoinHandle<()> {
        let hub = Arc::clone(self);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval.max(Duration::from_secs(15)));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                ticker.tick().await;
                let _ = hub.proactive_refresh_expiring(margin_secs).await;
            }
        })
    }

    async fn refresh_account_internal(
        &self,
        account_key: &str,
        skip_if_expires_after: Option<i64>,
    ) -> Result<GatewayAccountView, GatewayError> {
        let lock = self.refresh_lock_for(account_key);
        let _single_flight = lock.lock().await;

        let (vendor, current_bundle, current_expires_at) = {
            let guard = self.snapshot.read().unwrap_or_else(|p| p.into_inner());
            let acc = guard
                .accounts
                .get(account_key)
                .ok_or(GatewayError::AccountNotFound)?;
            let bundle = acc
                .bundle
                .clone()
                .ok_or(GatewayError::AccountReauthRequired)?;
            (acc.vendor, bundle, acc.expires_at)
        };

        // Double-checked locking: if another task already refreshed this account, return immediately.
        if let (Some(threshold), Some(exp)) = (skip_if_expires_after, current_expires_at)
            && exp > threshold
        {
            return self.get_account_view(account_key).await;
        }

        let driver = self.driver_for(vendor)?;
        match driver.refresh_token(&current_bundle).await {
            Ok(exchanged) => {
                self.persist_refreshed_bundle(account_key, vendor, exchanged)
                    .await?;
                self.get_account_view(account_key).await
            }
            Err(err) => {
                self.mark_account_reauth_required(account_key, err.code())
                    .await;
                Err(err)
            }
        }
    }

    async fn persist_refreshed_bundle(
        &self,
        account_key: &str,
        vendor: GatewayVendor,
        exchanged: ExchangedOAuthAccount,
    ) -> Result<(), GatewayError> {
        let credential_json = exchanged
            .bundle
            .seal(self.secret_resolver.as_ref(), account_key)?;
        let now_epoch = unix_seconds().unwrap_or(0);
        let extra_headers = self
            .drivers
            .get(&vendor)
            .map(|drv| drv.extra_headers_for(&exchanged.bundle))
            .unwrap_or_default();

        if let Some(pool) = &self.pool {
            sqlx::query(
                "UPDATE llm_gateway_accounts \
                 SET credential_json = ?, \
                     account_email = COALESCE(?, account_email), \
                     expires_at = ?, \
                     status = 'active', \
                     last_error = NULL, \
                     last_refreshed_at = ?, \
                     updated_at = ? \
                 WHERE account_key = ?",
            )
            .bind(&credential_json)
            .bind(&exchanged.account_email)
            .bind(exchanged.expires_at)
            .bind(now_epoch)
            .bind(now_epoch)
            .bind(account_key)
            .execute(pool)
            .await
            .map_err(|_| GatewayError::DatabaseUnavailable)?;
        }

        let mut guard = self.snapshot.write().unwrap_or_else(|p| p.into_inner());
        if let Some(acc) = guard.accounts.get_mut(account_key) {
            acc.credential_json = credential_json;
            acc.bundle = Some(exchanged.bundle);
            acc.extra_headers = extra_headers;
            if exchanged.account_email.is_some() {
                acc.account_email = exchanged.account_email;
            }
            acc.expires_at = exchanged.expires_at;
            acc.status = GatewayAccountStatus::Active;
            acc.last_error = None;
            acc.last_refreshed_at = Some(now_epoch);
            acc.cooldown_until = None;
        }
        Ok(())
    }

    async fn mark_account_reauth_required(&self, account_key: &str, err_code: &str) {
        let now_epoch = unix_seconds().unwrap_or(0);
        if let Some(pool) = &self.pool {
            let _ = sqlx::query(
                "UPDATE llm_gateway_accounts \
                 SET status = 'reauth_required', last_error = ?, updated_at = ? \
                 WHERE account_key = ?",
            )
            .bind(err_code)
            .bind(now_epoch)
            .bind(account_key)
            .execute(pool)
            .await;
        }
        let mut guard = self.snapshot.write().unwrap_or_else(|p| p.into_inner());
        if let Some(acc) = guard.accounts.get_mut(account_key) {
            acc.status = GatewayAccountStatus::ReauthRequired;
            acc.last_error = Some(err_code.to_owned());
        }
    }

    pub fn list_vendors(&self) -> Vec<GatewayVendorInfo> {
        let now = Instant::now();
        let guard = self.snapshot.read().unwrap_or_else(|p| p.into_inner());
        GatewayVendor::ALL
            .into_iter()
            .map(|vendor| {
                let vendor_accounts: Vec<&LoadedGatewayAccount> = guard
                    .accounts
                    .values()
                    .filter(|a| a.vendor == vendor)
                    .collect();
                let total_accounts = vendor_accounts.len();
                let cooling_down_accounts = vendor_accounts
                    .iter()
                    .filter(|a| a.cooldown_remaining_secs(now) > 0)
                    .count();
                let active_accounts = vendor_accounts
                    .iter()
                    .filter(|a| a.is_ready_for_lease(now))
                    .count();
                GatewayVendorInfo {
                    vendor,
                    display_name: vendor.display_name().to_owned(),
                    login_mode: vendor.login_mode(),
                    default_base_url: vendor.default_base_url().to_owned(),
                    pool_binding: GatewayBinding::Pool(vendor).as_config_string(),
                    total_accounts,
                    active_accounts,
                    cooling_down_accounts,
                }
            })
            .collect()
    }

    pub async fn list_accounts(
        &self,
        vendor_filter: Option<GatewayVendor>,
    ) -> Result<Vec<GatewayAccountView>, GatewayError> {
        let bound_map = self.load_bound_providers().await;
        let now = Instant::now();
        let guard = self.snapshot.read().unwrap_or_else(|p| p.into_inner());
        let quotas_guard = self.quotas.read().unwrap_or_else(|p| p.into_inner());
        let mut list: Vec<&LoadedGatewayAccount> = guard
            .accounts
            .values()
            .filter(|acc| vendor_filter.is_none_or(|v| acc.vendor == v))
            .collect();
        list.sort_by(|a, b| {
            a.vendor
                .cmp(&b.vendor)
                .then_with(|| a.priority.cmp(&b.priority))
                .then_with(|| a.account_key.cmp(&b.account_key))
        });

        Ok(list
            .into_iter()
            .map(|acc| {
                let cached_quota = quotas_guard.get(&acc.account_key).cloned();
                Self::to_account_view(acc, now, &bound_map, cached_quota)
            })
            .collect())
    }

    async fn get_account_view(
        &self,
        account_key: &str,
    ) -> Result<GatewayAccountView, GatewayError> {
        let bound_map = self.load_bound_providers().await;
        let now = Instant::now();
        let guard = self.snapshot.read().unwrap_or_else(|p| p.into_inner());
        let quotas_guard = self.quotas.read().unwrap_or_else(|p| p.into_inner());
        let acc = guard
            .accounts
            .get(account_key)
            .ok_or(GatewayError::AccountNotFound)?;
        let cached_quota = quotas_guard.get(account_key).cloned();
        Ok(Self::to_account_view(acc, now, &bound_map, cached_quota))
    }

    fn to_account_view(
        acc: &LoadedGatewayAccount,
        now: Instant,
        bound_map: &HashMap<String, Vec<GatewayBoundProviderRef>>,
        quota: Option<GatewayAccountQuota>,
    ) -> GatewayAccountView {
        let mut bound = bound_map
            .get(&format!("account:{}", acc.account_key))
            .cloned()
            .unwrap_or_default();
        if let Some(pool_bound) = bound_map.get(&format!("pool:{}", acc.vendor.as_str())) {
            bound.extend(pool_bound.iter().cloned());
        }
        GatewayAccountView {
            account_key: acc.account_key.clone(),
            vendor: acc.vendor,
            label: acc.label.clone(),
            account_email: acc.account_email.clone(),
            credential_metadata: credentials::metadata(Some(&acc.credential_json)),
            enabled: acc.enabled,
            priority: acc.priority,
            status: acc.status,
            cooldown_remaining_secs: acc.cooldown_remaining_secs(now),
            expires_at: acc.expires_at,
            last_refreshed_at: acc.last_refreshed_at,
            last_used_at: acc.last_used_at,
            last_error: acc.last_error.clone(),
            bound_providers: bound,
            quota,
        }
    }

    async fn load_bound_providers(&self) -> HashMap<String, Vec<GatewayBoundProviderRef>> {
        let mut map: HashMap<String, Vec<GatewayBoundProviderRef>> = HashMap::new();
        let Some(pool) = &self.pool else {
            return map;
        };
        let Ok(rows) = sqlx::query_as::<_, (String, String, String)>(
            "SELECT key, name, config_json FROM providers WHERE type = 'llm' ORDER BY id ASC",
        )
        .fetch_all(pool)
        .await
        else {
            return map;
        };
        for (key, name, config_json) in rows {
            let Ok(val) = serde_json::from_str::<Value>(&config_json) else {
                continue;
            };
            let Some(binding_raw) = val.get("gateway_binding").and_then(Value::as_str) else {
                continue;
            };
            let Ok(binding) = GatewayBinding::parse(binding_raw) else {
                continue;
            };
            let (canonical, mode) = match &binding {
                GatewayBinding::Account(_) => (binding.as_config_string(), "account"),
                GatewayBinding::Pool(_) => (binding.as_config_string(), "pool"),
            };
            map.entry(canonical).or_default().push(GatewayBoundProviderRef {
                provider_key: key,
                display_name: name,
                binding_mode: mode.to_owned(),
            });
        }
        map
    }

    pub async fn start_oauth_login(
        &self,
        req: StartOAuthLoginRequest,
    ) -> Result<OAuthLoginChallenge, GatewayError> {
        let label = req.label.trim();
        if label.is_empty() || label.len() > 256 {
            return Err(GatewayError::ValidationFailed);
        }
        let driver = self.driver_for(req.vendor)?;
        let session_id = format!("oas_{}", uuid::Uuid::new_v4().simple());
        let now = Instant::now();
        self.prune_expired_sessions(now);

        match req.vendor.login_mode() {
            OAuthLoginMode::Pkce => {
                let (code_verifier, code_challenge) = generate_pkce_pair()?;
                let state = generate_state_token()?;
                let redirect_uri = driver.resolve_redirect_uri(req.redirect_uri.as_deref());
                let authorize_url =
                    driver.build_pkce_authorize_url(&state, &code_challenge, &redirect_uri)?;
                let session = OAuthLoginSession {
                    session_id: session_id.clone(),
                    vendor: req.vendor,
                    label: label.to_owned(),
                    created_at: now,
                    expires_at: now + OAUTH_SESSION_TTL,
                    flow: OAuthSessionFlowState::Pkce {
                        state,
                        code_verifier,
                        redirect_uri,
                        completed_account: None,
                        failure: None,
                    },
                };
                self.sessions
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .insert(session_id.clone(), session);
                Ok(OAuthLoginChallenge {
                    session_id,
                    vendor: req.vendor,
                    mode: OAuthLoginMode::Pkce,
                    authorize_url: Some(authorize_url),
                    verification_uri: None,
                    user_code: None,
                    poll_interval_secs: 2,
                    expires_in_secs: OAUTH_SESSION_TTL.as_secs(),
                })
            }
            OAuthLoginMode::DeviceCode => {
                let grant = driver.start_device_flow().await?;
                let ttl = Duration::from_secs(grant.expires_in_secs.clamp(60, 1800));
                let session = OAuthLoginSession {
                    session_id: session_id.clone(),
                    vendor: req.vendor,
                    label: label.to_owned(),
                    created_at: now,
                    expires_at: now + ttl,
                    flow: OAuthSessionFlowState::DeviceCode {
                        device_code: grant.device_code,
                        user_code: grant.user_code.clone(),
                        verification_uri: grant.verification_uri.clone(),
                        poll_interval_secs: grant.interval_secs.max(1),
                        completed_account: None,
                        failure: None,
                    },
                };
                self.sessions
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .insert(session_id.clone(), session);
                Ok(OAuthLoginChallenge {
                    session_id,
                    vendor: req.vendor,
                    mode: OAuthLoginMode::DeviceCode,
                    authorize_url: None,
                    verification_uri: Some(grant.verification_uri),
                    user_code: Some(grant.user_code),
                    poll_interval_secs: grant.interval_secs.max(1),
                    expires_in_secs: ttl.as_secs(),
                })
            }
        }
    }

    pub async fn complete_oauth_login(
        &self,
        session_id: &str,
        submission: OAuthCallbackSubmission,
    ) -> Result<GatewayAccountView, GatewayError> {
        let (code, submitted_state) = parse_oauth_callback_submission(&submission)?;
        let (vendor, label, expected_state, code_verifier, redirect_uri) = {
            let mut guard = self.sessions.lock().unwrap_or_else(|p| p.into_inner());
            let session = guard
                .get_mut(session_id)
                .ok_or(GatewayError::OAuthSessionNotFound)?;
            if session.is_expired(Instant::now()) {
                guard.remove(session_id);
                return Err(GatewayError::OAuthSessionExpired);
            }
            match &session.flow {
                OAuthSessionFlowState::Pkce {
                    state,
                    code_verifier,
                    redirect_uri,
                    completed_account: Some(acc),
                    ..
                } => {
                    let _ = (state, code_verifier, redirect_uri);
                    return Ok(acc.clone());
                }
                OAuthSessionFlowState::Pkce {
                    state,
                    code_verifier,
                    redirect_uri,
                    completed_account: None,
                    ..
                } => (
                    session.vendor,
                    session.label.clone(),
                    state.clone(),
                    code_verifier.clone(),
                    redirect_uri.clone(),
                ),
                OAuthSessionFlowState::DeviceCode { .. } => {
                    return Err(GatewayError::OAuthCallbackInvalid);
                }
            }
        };

        if let Some(given_state) = submitted_state.as_deref()
            && given_state != expected_state
        {
            return Err(GatewayError::OAuthStateMismatch);
        }

        let driver = self.driver_for(vendor)?;
        let exchanged = driver
            .exchange_pkce_code(&code, &expected_state, &code_verifier, &redirect_uri)
            .await?;
        let account_view = self.save_new_account(vendor, &label, exchanged).await?;

        let mut guard = self.sessions.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(session) = guard.get_mut(session_id)
            && let OAuthSessionFlowState::Pkce {
                completed_account, ..
            } = &mut session.flow
        {
            *completed_account = Some(account_view.clone());
        }
        Ok(account_view)
    }

    pub async fn complete_oauth_by_state(
        &self,
        state_param: &str,
        code: &str,
    ) -> Result<(String, GatewayAccountView), GatewayError> {
        let session_id = {
            let guard = self.sessions.lock().unwrap_or_else(|p| p.into_inner());
            guard
                .values()
                .find_map(|s| match &s.flow {
                    OAuthSessionFlowState::Pkce { state, .. } if state == state_param => {
                        Some(s.session_id.clone())
                    }
                    _ => None,
                })
                .ok_or(GatewayError::OAuthSessionNotFound)?
        };
        match self
            .complete_oauth_login(
                &session_id,
                OAuthCallbackSubmission {
                    callback_url_or_code: code.to_owned(),
                    state: Some(state_param.to_owned()),
                },
            )
            .await
        {
            Ok(view) => Ok((session_id, view)),
            Err(err) => {
                self.record_oauth_failure(&session_id, err.to_string());
                Err(err)
            }
        }
    }

    pub fn record_oauth_failure_by_state(&self, state_param: &str, message: impl Into<String>) {
        let msg = message.into();
        let mut guard = self.sessions.lock().unwrap_or_else(|p| p.into_inner());
        for session in guard.values_mut() {
            if let OAuthSessionFlowState::Pkce {
                state,
                completed_account: None,
                failure,
                ..
            } = &mut session.flow
                && state == state_param
            {
                *failure = Some(msg);
                break;
            }
        }
    }

    fn record_oauth_failure(&self, session_id: &str, message: String) {
        let mut guard = self.sessions.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(session) = guard.get_mut(session_id)
            && let OAuthSessionFlowState::Pkce {
                completed_account: None,
                failure,
                ..
            } = &mut session.flow
        {
            *failure = Some(message);
        }
    }

    pub async fn poll_oauth_login(&self, session_id: &str) -> Result<OAuthPollStatus, GatewayError> {
        enum Action {
            Return(OAuthPollStatus),
            PollDevice {
                vendor: GatewayVendor,
                label: String,
                device_code: String,
                poll_interval_secs: u64,
            },
        }

        let action = {
            let mut guard = self.sessions.lock().unwrap_or_else(|p| p.into_inner());
            let session = guard
                .get_mut(session_id)
                .ok_or(GatewayError::OAuthSessionNotFound)?;
            if session.is_expired(Instant::now()) {
                guard.remove(session_id);
                return Err(GatewayError::OAuthSessionExpired);
            }
            match &session.flow {
                OAuthSessionFlowState::Pkce {
                    completed_account: Some(acc),
                    ..
                }
                | OAuthSessionFlowState::DeviceCode {
                    completed_account: Some(acc),
                    ..
                } => Action::Return(OAuthPollStatus::Completed {
                    account: acc.clone(),
                }),
                OAuthSessionFlowState::Pkce {
                    failure: Some(err), ..
                }
                | OAuthSessionFlowState::DeviceCode {
                    failure: Some(err), ..
                } => Action::Return(OAuthPollStatus::Failed { error: err.clone() }),
                OAuthSessionFlowState::Pkce { .. } => Action::Return(OAuthPollStatus::Pending {
                    poll_interval_secs: 2,
                }),
                OAuthSessionFlowState::DeviceCode {
                    device_code,
                    poll_interval_secs,
                    ..
                } => Action::PollDevice {
                    vendor: session.vendor,
                    label: session.label.clone(),
                    device_code: device_code.clone(),
                    poll_interval_secs: *poll_interval_secs,
                },
            }
        };

        match action {
            Action::Return(status) => Ok(status),
            Action::PollDevice {
                vendor,
                label,
                device_code,
                poll_interval_secs,
            } => {
                let driver = self.driver_for(vendor)?;
                match driver.poll_device_flow(&device_code).await? {
                    DevicePollOutcome::Pending { next_interval_secs } => {
                        let next_interval = next_interval_secs.unwrap_or(poll_interval_secs).max(1);
                        let mut guard = self.sessions.lock().unwrap_or_else(|p| p.into_inner());
                        if let Some(session) = guard.get_mut(session_id)
                            && let OAuthSessionFlowState::DeviceCode {
                                poll_interval_secs: ref mut stored,
                                ..
                            } = session.flow
                        {
                            *stored = next_interval;
                        }
                        Ok(OAuthPollStatus::Pending {
                            poll_interval_secs: next_interval,
                        })
                    }
                    DevicePollOutcome::Completed(exchanged) => {
                        let view = self.save_new_account(vendor, &label, exchanged).await?;
                        let mut guard = self.sessions.lock().unwrap_or_else(|p| p.into_inner());
                        if let Some(session) = guard.get_mut(session_id)
                            && let OAuthSessionFlowState::DeviceCode {
                                completed_account, ..
                            } = &mut session.flow
                        {
                            *completed_account = Some(view.clone());
                        }
                        Ok(OAuthPollStatus::Completed { account: view })
                    }
                    DevicePollOutcome::Failed(err) => {
                        let mut guard = self.sessions.lock().unwrap_or_else(|p| p.into_inner());
                        if let Some(session) = guard.get_mut(session_id)
                            && let OAuthSessionFlowState::DeviceCode { failure, .. } =
                                &mut session.flow
                        {
                            *failure = Some(err.clone());
                        }
                        Ok(OAuthPollStatus::Failed { error: err })
                    }
                }
            }
        }
    }

    async fn save_new_account(
        &self,
        vendor: GatewayVendor,
        label: &str,
        exchanged: ExchangedOAuthAccount,
    ) -> Result<GatewayAccountView, GatewayError> {
        let short_id = &uuid::Uuid::new_v4().simple().to_string()[..8];
        let account_key = format!("gw_{}_{short_id}", vendor.as_str());
        let credential_json = exchanged
            .bundle
            .seal(self.secret_resolver.as_ref(), &account_key)?;
        let now_epoch = unix_seconds().unwrap_or(0);
        let extra_headers = self
            .drivers
            .get(&vendor)
            .map(|drv| drv.extra_headers_for(&exchanged.bundle))
            .unwrap_or_default();

        if let Some(pool) = &self.pool {
            sqlx::query(
                "INSERT INTO llm_gateway_accounts (
                    account_key, vendor, label, account_email, credential_json,
                    expires_at, enabled, priority, status, last_refreshed_at, created_at, updated_at
                 ) VALUES (?, ?, ?, ?, ?, ?, 1, 100, 'active', ?, ?, ?)",
            )
            .bind(&account_key)
            .bind(vendor.as_str())
            .bind(label)
            .bind(&exchanged.account_email)
            .bind(&credential_json)
            .bind(exchanged.expires_at)
            .bind(now_epoch)
            .bind(now_epoch)
            .bind(now_epoch)
            .execute(pool)
            .await
            .map_err(|_| GatewayError::DatabaseUnavailable)?;
        }

        {
            let mut guard = self.snapshot.write().unwrap_or_else(|p| p.into_inner());
            guard.accounts.insert(
                account_key.clone(),
                LoadedGatewayAccount {
                    account_key: account_key.clone(),
                    vendor,
                    label: label.to_owned(),
                    account_email: exchanged.account_email,
                    credential_json,
                    bundle: Some(exchanged.bundle),
                    extra_headers,
                    expires_at: exchanged.expires_at,
                    enabled: true,
                    priority: 100,
                    status: GatewayAccountStatus::Active,
                    last_error: None,
                    last_refreshed_at: Some(now_epoch),
                    last_used_at: None,
                    cooldown_until: None,
                },
            );
            guard.rebuild_vendor_index();
        }

        self.get_account_view(&account_key).await
    }

    pub async fn refresh_account_now(
        &self,
        account_key: &str,
    ) -> Result<GatewayAccountView, GatewayError> {
        if !valid_account_key(account_key) {
            return Err(GatewayError::AccountNotFound);
        }
        self.refresh_account_internal(account_key, None).await
    }

    pub async fn get_account_quota(
        &self,
        account_key: &str,
        force: bool,
    ) -> Result<GatewayAccountQuota, GatewayError> {
        if !valid_account_key(account_key) {
            return Err(GatewayError::AccountNotFound);
        }
        let now_epoch = unix_seconds().unwrap_or(0);
        if !force {
            let guard = self.quotas.read().unwrap_or_else(|p| p.into_inner());
            if let Some(cached) = guard.get(account_key)
                && now_epoch.saturating_sub(cached.fetched_at) < QUOTA_CACHE_TTL_SECS
            {
                return Ok(cached.clone());
            }
        }

        let (vendor, expires_at, has_refresh) = {
            let guard = self.snapshot.read().unwrap_or_else(|p| p.into_inner());
            let acc = guard
                .accounts
                .get(account_key)
                .ok_or(GatewayError::AccountNotFound)?;
            let has_refresh = acc
                .bundle
                .as_ref()
                .is_some_and(|b| b.refresh_token.is_some() || b.upstream_oauth_token.is_some());
            (acc.vendor, acc.expires_at, has_refresh)
        };

        if has_refresh && expires_at.is_some_and(|exp| exp <= now_epoch + 60) {
            let _ = self
                .refresh_account_internal(account_key, Some(now_epoch + 60))
                .await;
        }

        let bundle = {
            let guard = self.snapshot.read().unwrap_or_else(|p| p.into_inner());
            let acc = guard
                .accounts
                .get(account_key)
                .ok_or(GatewayError::AccountNotFound)?;
            acc.bundle
                .clone()
                .ok_or(GatewayError::AccountReauthRequired)?
        };

        let driver = self.driver_for(vendor)?;
        let snapshot_res = match driver.fetch_quota(&bundle).await {
            Err(GatewayError::AccountReauthRequired) if has_refresh => {
                self.refresh_account_internal(account_key, None).await?;
                let refreshed_bundle = {
                    let guard = self.snapshot.read().unwrap_or_else(|p| p.into_inner());
                    guard
                        .accounts
                        .get(account_key)
                        .and_then(|acc| acc.bundle.clone())
                        .ok_or(GatewayError::AccountReauthRequired)?
                };
                driver.fetch_quota(&refreshed_bundle).await
            }
            other => other,
        };

        let vendor_snapshot = match snapshot_res {
            Ok(snap) => snap,
            Err(err) => {
                if matches!(err, GatewayError::AccountReauthRequired) {
                    self.mark_account_reauth_required(account_key, err.code())
                        .await;
                }
                return Err(err);
            }
        };

        if let Some(ref discovered_ctx) = vendor_snapshot.discovered_account_context_id
            && bundle.account_context_id.is_none()
        {
            self.persist_discovered_context_id(account_key, discovered_ctx)
                .await;
        }

        let quota = GatewayAccountQuota {
            account_key: account_key.to_owned(),
            vendor,
            plan: vendor_snapshot.plan,
            limit_reached: vendor_snapshot.limit_reached,
            windows: vendor_snapshot.windows,
            message: vendor_snapshot.message,
            fetched_at: unix_seconds().unwrap_or(now_epoch),
        };

        self.quotas
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .insert(account_key.to_owned(), quota.clone());
        Ok(quota)
    }

    pub async fn list_quotas(
        &self,
        vendor_filter: Option<GatewayVendor>,
        force: bool,
    ) -> Result<Vec<GatewayAccountQuota>, GatewayError> {
        let targets: Vec<(String, GatewayVendor)> = {
            let guard = self.snapshot.read().unwrap_or_else(|p| p.into_inner());
            let mut list: Vec<&LoadedGatewayAccount> = guard
                .accounts
                .values()
                .filter(|acc| vendor_filter.is_none_or(|v| acc.vendor == v))
                .collect();
            list.sort_by(|a, b| {
                a.vendor
                    .cmp(&b.vendor)
                    .then_with(|| a.priority.cmp(&b.priority))
                    .then_with(|| a.account_key.cmp(&b.account_key))
            });
            list.into_iter()
                .map(|acc| (acc.account_key.clone(), acc.vendor))
                .collect()
        };

        let now_epoch = unix_seconds().unwrap_or(0);
        let mut results = Vec::with_capacity(targets.len());
        for (account_key, vendor) in targets {
            match self.get_account_quota(&account_key, force).await {
                Ok(q) => results.push(q),
                Err(err) => {
                    let fallback = GatewayAccountQuota {
                        account_key: account_key.clone(),
                        vendor,
                        plan: None,
                        limit_reached: false,
                        windows: Vec::new(),
                        message: Some(err.to_string()),
                        fetched_at: now_epoch,
                    };
                    results.push(fallback);
                }
            }
        }
        Ok(results)
    }

    async fn persist_discovered_context_id(&self, account_key: &str, context_id: &str) {
        let updated_bundle = {
            let guard = self.snapshot.read().unwrap_or_else(|p| p.into_inner());
            let Some(acc) = guard.accounts.get(account_key) else {
                return;
            };
            let Some(mut b) = acc.bundle.clone() else {
                return;
            };
            b.account_context_id = Some(context_id.to_owned());
            b
        };
        let Ok(credential_json) = updated_bundle.seal(self.secret_resolver.as_ref(), account_key)
        else {
            return;
        };
        if let Some(pool) = &self.pool {
            let _ = sqlx::query(
                "UPDATE llm_gateway_accounts SET credential_json = ? WHERE account_key = ?",
            )
            .bind(&credential_json)
            .bind(account_key)
            .execute(pool)
            .await;
        }
        let mut guard = self.snapshot.write().unwrap_or_else(|p| p.into_inner());
        if let Some(acc) = guard.accounts.get_mut(account_key) {
            acc.credential_json = credential_json;
            acc.bundle = Some(updated_bundle);
        }
    }

    pub async fn clear_cooldown(
        &self,
        account_key: &str,
    ) -> Result<GatewayAccountView, GatewayError> {
        {
            let mut guard = self.snapshot.write().unwrap_or_else(|p| p.into_inner());
            let acc = guard
                .accounts
                .get_mut(account_key)
                .ok_or(GatewayError::AccountNotFound)?;
            acc.cooldown_until = None;
            acc.last_error = None;
        }
        self.get_account_view(account_key).await
    }

    pub async fn update_account(
        &self,
        account_key: &str,
        patch: UpdateGatewayAccountPatch,
    ) -> Result<GatewayAccountView, GatewayError> {
        let (next_label, next_enabled, next_priority, next_status) = {
            let guard = self.snapshot.read().unwrap_or_else(|p| p.into_inner());
            let acc = guard
                .accounts
                .get(account_key)
                .ok_or(GatewayError::AccountNotFound)?;
            let label = match patch.label {
                Some(ref l) => {
                    let trimmed = l.trim();
                    if trimmed.is_empty() || trimmed.len() > 256 {
                        return Err(GatewayError::ValidationFailed);
                    }
                    trimmed.to_owned()
                }
                None => acc.label.clone(),
            };
            let enabled = patch.enabled.unwrap_or(acc.enabled);
            let priority = patch.priority.unwrap_or(acc.priority);
            if !(0..=10_000).contains(&priority) {
                return Err(GatewayError::ValidationFailed);
            }
            let status = if !enabled {
                GatewayAccountStatus::Disabled
            } else if acc.status == GatewayAccountStatus::Disabled {
                if acc.bundle.is_some() {
                    GatewayAccountStatus::Active
                } else {
                    GatewayAccountStatus::ReauthRequired
                }
            } else {
                acc.status
            };
            (label, enabled, priority, status)
        };

        let now_epoch = unix_seconds().unwrap_or(0);
        if let Some(pool) = &self.pool {
            sqlx::query(
                "UPDATE llm_gateway_accounts \
                 SET label = ?, enabled = ?, priority = ?, status = ?, updated_at = ? \
                 WHERE account_key = ?",
            )
            .bind(&next_label)
            .bind(i64::from(next_enabled))
            .bind(next_priority)
            .bind(next_status.as_str())
            .bind(now_epoch)
            .bind(account_key)
            .execute(pool)
            .await
            .map_err(|_| GatewayError::DatabaseUnavailable)?;
        }

        {
            let mut guard = self.snapshot.write().unwrap_or_else(|p| p.into_inner());
            if let Some(acc) = guard.accounts.get_mut(account_key) {
                acc.label = next_label;
                acc.enabled = next_enabled;
                acc.priority = next_priority;
                acc.status = next_status;
            }
            guard.rebuild_vendor_index();
        }

        self.get_account_view(account_key).await
    }

    pub async fn delete_account(&self, account_key: &str) -> Result<(), GatewayError> {
        let bound_map = self.load_bound_providers().await;
        if let Some(bound_list) = bound_map.get(&format!("account:{account_key}"))
            && !bound_list.is_empty()
        {
            return Err(GatewayError::AccountInUse(
                bound_list.iter().map(|b| b.provider_key.clone()).collect(),
            ));
        }

        {
            let guard = self.snapshot.read().unwrap_or_else(|p| p.into_inner());
            if !guard.accounts.contains_key(account_key) {
                return Err(GatewayError::AccountNotFound);
            }
        }

        if let Some(pool) = &self.pool {
            sqlx::query("DELETE FROM llm_gateway_accounts WHERE account_key = ?")
                .bind(account_key)
                .execute(pool)
                .await
                .map_err(|_| GatewayError::DatabaseUnavailable)?;
        }

        {
            let mut guard = self.snapshot.write().unwrap_or_else(|p| p.into_inner());
            guard.accounts.remove(account_key);
            guard.rebuild_vendor_index();
        }
        self.quotas
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .remove(account_key);
        self.refresh_locks
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(account_key);
        Ok(())
    }

    fn prune_expired_sessions(&self, now: Instant) {
        let mut guard = self.sessions.lock().unwrap_or_else(|p| p.into_inner());
        guard.retain(|_, session| !session.is_expired(now));
    }
}

static GLOBAL_GATEWAY_HUB: OnceLock<RwLock<Arc<LlmGatewayHub>>> = OnceLock::new();

/// Process-wide access point matching `SharedMusePool` so `COMPILED_PROVIDER_REGISTRY`
/// (`LlmFactory::build(&self, config: &LlmInstanceConfig)`) can construct `LlmGatewayProvider`
/// without altering static factory signatures.
pub struct SharedGatewayHub;

impl SharedGatewayHub {
    fn cell() -> &'static RwLock<Arc<LlmGatewayHub>> {
        GLOBAL_GATEWAY_HUB.get_or_init(|| {
            RwLock::new(Arc::new(LlmGatewayHub::new(
                None,
                Arc::new(EnvSecretResolver),
            )))
        })
    }

    pub fn global() -> Arc<LlmGatewayHub> {
        Self::cell()
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    pub fn set_global(hub: Arc<LlmGatewayHub>) {
        let mut guard = Self::cell().write().unwrap_or_else(|p| p.into_inner());
        *guard = hub;
    }
}
