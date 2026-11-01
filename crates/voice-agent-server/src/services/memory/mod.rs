//! Memory Core: the only read and write path for Memory (ADR 0135). A per-Device Markdown vault,
//! separate from the Wiki. This file is the interface; the logic lives in the private modules.

mod apply;
mod fact_file;
mod fit_summary;
mod guard;
mod index;
mod intent;
mod normalize;
mod read;
mod reconcile;
mod scopes;
mod tombstone;
mod types;
mod vault;

#[cfg(test)]
mod tests;

pub use intent::classify_intent;
pub use types::{
    DeviceKind, ExplicitIntent, FactId, FactKind, FactOrigin, FactStatus, InspectFilter,
    MemoryActor, MemoryChange, MemoryContext, MemoryContextHandle, MemoryError, MemoryFact,
    MemoryMode, MemoryOverview, MemoryProfile, MemoryReceipt, MemoryScope, Provenance, RecallQuery,
    RecallResult, ReceiptAction, ReconcileAllReport, ReconcileReport,
};

use crate::config::MemorySectionConfig;
use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use types::ScopeTarget;

const CONTEXT_FIRST_LOAD_DEADLINE: Duration = Duration::from_millis(150);

struct Inner {
    vault: vault::Vault,
    key: [u8; 32],
    voice_summary_max_bytes: usize,
    max_facts_per_scope: usize,
    max_fact_body_chars: usize,
    registry: scopes::Registry,
    #[cfg(test)]
    fail_point: std::sync::Mutex<Option<(apply::FailPoint, apply::FailAction)>>,
}

#[derive(Clone)]
pub struct MemoryCore {
    inner: Arc<Inner>,
}

impl MemoryCore {
    /// Opens the vault and creates the tombstone key on first use. A disabled configuration is
    /// refused, so `[memory].enabled = false` never touches the disk.
    pub fn open(cfg: &MemorySectionConfig) -> Result<Self, MemoryError> {
        if !cfg.enabled {
            return Err(MemoryError::Disabled);
        }
        let vault = vault::Vault::new(&cfg.vault_dir);
        vault::ensure_dir(&vault.devices_dir()).map_err(read::unavailable)?;
        vault::ensure_dir(&vault.deployment_dir()).map_err(read::unavailable)?;
        let key =
            vault::load_or_create_key(&vault.tombstone_key_path()).map_err(read::unavailable)?;
        Ok(Self {
            inner: Arc::new(Inner {
                vault,
                key,
                voice_summary_max_bytes: cfg.voice_summary_max_bytes,
                max_facts_per_scope: cfg.max_facts_per_scope,
                max_fact_body_chars: cfg.max_fact_body_chars,
                registry: scopes::Registry::default(),
                #[cfg(test)]
                fail_point: std::sync::Mutex::new(None),
            }),
        })
    }

    /// Never fails, so it cannot break admission. The first load waits at most 150 ms; past that the
    /// handle starts empty and fills in by itself when the load finishes.
    pub async fn context(&self, scope: &MemoryScope) -> MemoryContextHandle {
        let key = vault::Vault::scope_key(scope);
        let entry = self.inner.registry.entry(&key);
        let handle = MemoryContextHandle(entry.context.subscribe());
        if entry.loaded.load(Ordering::Acquire) {
            return handle;
        }
        let inner = self.inner.clone();
        let is_deployment = matches!(scope.0, ScopeTarget::Deployment);
        let load = tokio::spawn(async move {
            let _writer = entry.writer.lock().await;
            if entry.loaded.load(Ordering::Acquire) {
                return;
            }
            // Both publish an empty context on failure, and `loaded` stays false so a later call retries.
            if is_deployment {
                let _ = read::publish_deployment(&inner, &entry).await;
            } else {
                let _ = read::publish_from_disk(&inner, &key, &entry).await;
            }
        });
        if tokio::time::timeout(CONTEXT_FIRST_LOAD_DEADLINE, load)
            .await
            .is_err()
        {
            tracing::warn!(
                event = "memory_context_degraded",
                reason = "first_load_deadline"
            );
        }
        handle
    }

    /// Searches, then re-checks every candidate against the real file. Results are untrusted data.
    pub async fn recall(
        &self,
        scope: &MemoryScope,
        query: RecallQuery,
    ) -> Result<RecallResult, MemoryError> {
        read::recall(&self.inner, scope, query).await
    }

    /// The only write path. The body runs in its own task, so dropping this future (barge-in) cannot
    /// stop a write sequence halfway. On success the change is already visible to every `context`
    /// and `recall`.
    pub async fn apply(
        &self,
        scope: &MemoryScope,
        actor: MemoryActor,
        change: MemoryChange,
    ) -> Result<MemoryReceipt, MemoryError> {
        let task = apply::apply(self.inner.clone(), scope.clone(), actor, change);
        tokio::spawn(task)
            .await
            .map_err(|_| MemoryError::Unavailable)?
    }

    /// Admin read, including quarantine counts. On the deployment scope, the profile.
    pub async fn inspect(
        &self,
        scope: &MemoryScope,
        filter: InspectFilter,
    ) -> Result<MemoryOverview, MemoryError> {
        read::inspect(&self.inner, scope, filter).await
    }

    /// Converges a vault after a crash or an outside edit, and rebuilds what is derived from it.
    pub async fn reconcile(&self, scope: &MemoryScope) -> Result<ReconcileReport, MemoryError> {
        reconcile::reconcile(&self.inner, scope).await
    }

    /// Runs at startup. Directories outside `known` are only reported, never deleted.
    pub async fn reconcile_all(&self, known: &[MemoryScope]) -> ReconcileAllReport {
        reconcile::reconcile_all(&self.inner, known).await
    }
}
