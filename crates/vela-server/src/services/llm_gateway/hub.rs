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
use tokio_util::sync::CancellationToken;

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
        CreatePoolModelRequest, GatewayAccountQuota, GatewayAccountStatus, GatewayAccountView,
        GatewayBinding, GatewayBoundProviderRef, GatewayCallOutcome, GatewayError, GatewayLease,
        GatewayPoolDetailView, GatewayPoolModelView, GatewayVendor, GatewayVendorInfo,
        ImportCookieAccountRequest, OAuthCallbackSubmission, OAuthLoginChallenge, OAuthLoginMode,
        OAuthPollStatus, OAuthTokenBundle, StartOAuthLoginRequest, UpdateGatewayAccountPatch,
        UpdatePoolModelPatch, valid_account_key,
    },
    vendors::{
        DevicePollOutcome, ExchangedOAuthAccount, OAuthVendorDriver, default_vendor_drivers,
    },
};

const DEFAULT_COOLDOWN_SECS: u64 = 60;
const MIN_COOLDOWN_SECS: u64 = 15;
const MAX_COOLDOWN_SECS: u64 = 600;
const QUOTA_CACHE_TTL_SECS: i64 = 60;
const MAX_PASTED_COOKIES_BYTES: usize = 256 * 1024;

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
        let mut guard = self.refresh_locks.lock().unwrap_or_else(|p| p.into_inner());
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

mod accounts;
mod lease;
mod oauth;
mod pools;
mod quotas;
mod refresh;
mod runtime_credentials;
