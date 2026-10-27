//! Bounded, coalesced desired-state hints. Mutations enqueue only after committing SQLite.
use crate::{
    database::{Database, DesiredProvider},
    services::provider_runtime::{ProviderRuntimeManager, ProviderVersion, ResourceKey},
};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicU64, Ordering},
    },
};
use tokio::sync::Notify;

#[derive(Clone, Hash, PartialEq, Eq)]
enum PrewarmKey {
    Resource(ResourceKey),
    Version(ProviderVersion),
}
pub struct ProviderPrewarm {
    database: Arc<Database>,
    manager: Weak<ProviderRuntimeManager>,
    intents: Arc<IntentQueue>,
    config: Arc<crate::config::AppConfig>,
    metrics: Arc<crate::services::provider_runtime::RuntimeMetrics>,
}
impl ProviderPrewarm {
    pub fn start(
        database: Arc<Database>,
        manager: &Arc<ProviderRuntimeManager>,
        config: Arc<crate::config::AppConfig>,
    ) -> Option<Arc<Self>> {
        let runtime = tokio::runtime::Handle::try_current().ok()?;
        let metrics = manager.metrics().clone();
        let intents = Arc::new(IntentQueue {
            pending: Mutex::new(HashMap::new()),
            epochs: Mutex::new(HashMap::new()),
            next_epoch: AtomicU64::new(1),
            notify: Notify::new(),
            capacity: (manager.prewarm_capacity() * 4).max(8),
            metrics: metrics.clone(),
        });
        let service = Arc::new(Self {
            database,
            config,
            manager: Arc::downgrade(manager),
            intents: intents.clone(),
            metrics,
        });
        let manager = service.manager.clone();
        let stopping = manager.upgrade()?.stopping_token();
        runtime.spawn(async move {
            loop {
                tokio::select! { _ = stopping.cancelled() => break, _ = intents.notify.notified() => {} }
                // Coalesce rapid edits without holding a DB transaction or delaying their HTTP response.
                tokio::select! { _ = stopping.cancelled() => break, _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {} }
                let snapshots = intents.take();
                let mut batch_leases = std::collections::VecDeque::with_capacity(4);
                for (key, (snapshot, epoch)) in snapshots {
                    if stopping.is_cancelled() { break; }
                    let Some(manager) = manager.upgrade() else { return };
                    // Pin at most the four most recently warmed resources while the batch runs,
                    // so a large batch cannot exhaust max_resources with its own earlier work.
                    // Pending intents are unordered, so this window does not follow templates.
                    while batch_leases.len() >= 4 {
                        batch_leases.pop_front();
                    }
                    // No inference operation or inference capacity is consumed here. Busy
                    // speculation (loader, waiters or the pilot envelope in use) is retried later
                    // with backoff; explicit acquisition remains available meanwhile.
                    match manager.acquire_speculative(snapshot.clone()).await {
                        Ok(lease) => {
                            intents.settle(&key, epoch);
                            batch_leases.push_back(lease);
                        }
                        Err(crate::services::provider_runtime::RuntimeError::Busy) => {
                            intents.busy(key, snapshot, epoch, stopping.clone());
                        }
                        Err(_) => intents.settle(&key, epoch),
                    }
                }
                drop(batch_leases);
            }
        });
        Some(service)
    }
    fn enqueue(&self, key: PrewarmKey, snapshot: DesiredProvider) -> bool {
        self.intents.offer(key, snapshot)
    }
    async fn enqueue_rows(&self, rows: Vec<DesiredProvider>) -> bool {
        if rows.is_empty() {
            self.metrics
                .increment(crate::services::provider_runtime::RuntimeCounter::ObsoleteIntent);
            return false;
        }
        let Some(manager) = self.manager.upgrade() else {
            return false;
        };
        let mut accepted = true;
        for snapshot in rows {
            let key = match manager.prewarm_resource_key(&snapshot) {
                Ok(Some(resource)) => PrewarmKey::Resource(resource),
                Ok(None) => PrewarmKey::Version(if snapshot.id == 0 {
                    ProviderVersion {
                        identity: crate::services::provider_runtime::ProviderIdentity::Deployment {
                            kind: snapshot.kind.clone(),
                            key: snapshot.key.clone(),
                        },
                        revision: snapshot.revision,
                    }
                } else {
                    ProviderVersion::database(snapshot.id, snapshot.revision)
                }),
                Err(_) => {
                    self.metrics.increment(
                        crate::services::provider_runtime::RuntimeCounter::ObsoleteIntent,
                    );
                    accepted = false;
                    continue;
                }
            };
            accepted &= self.enqueue(key, snapshot);
        }
        accepted
    }
    pub async fn provider(&self, id: i64, revision: i64) -> bool {
        self.enqueue_rows(snapshots(&self.database, Some((id, revision)), None).await)
            .await
    }
    pub async fn template(&self, id: i64) -> bool {
        let mut rows = snapshots(&self.database, None, Some(id)).await;
        if let Ok(bindings) = self.database.config_prewarm_bindings(id).await {
            rows.extend(bindings.into_iter().filter_map(|(kind, key, enabled)| {
                if enabled == 0 {
                    return None;
                }
                crate::providers::config_catalog::ProviderRef::parse(&key, Some(&kind))
                    .ok()?
                    .snapshot(&self.config)
                    .ok()
            }));
        }
        self.enqueue_rows(rows).await
    }
    pub async fn device(&self, device_id: &str) -> bool {
        match self.database.device_prewarm_template(device_id).await {
            Ok(Some(id)) => self.template(id).await,
            _ => false,
        }
    }
}
/// Busy speculation backs off 1s, 2s, 4s, … capped at 30s, and gives up after ten attempts
/// (about three minutes), long enough to outlast a typical voice turn in the pilot envelope.
fn retry_delay(attempt: u32) -> Option<std::time::Duration> {
    const MAX_ATTEMPTS: u32 = 10;
    (attempt <= MAX_ATTEMPTS).then(|| {
        let seconds = 1_u64 << attempt.saturating_sub(1).min(5);
        std::time::Duration::from_secs(seconds.min(30))
    })
}

/// Pending prewarm intents, coalesced per key. Every fresh intent gets a new epoch; a busy
/// retry re-queues only while its epoch is still current, so a retry can never resurrect an
/// older snapshot after a newer intent for the same key arrived.
struct IntentQueue {
    pending: Mutex<HashMap<PrewarmKey, (DesiredProvider, u64)>>,
    /// Current epoch and busy attempts of each unsettled intent. Lock order: pending, epochs.
    epochs: Mutex<HashMap<PrewarmKey, (u64, u32)>>,
    next_epoch: AtomicU64,
    notify: Notify,
    capacity: usize,
    metrics: Arc<crate::services::provider_runtime::RuntimeMetrics>,
}
impl IntentQueue {
    fn offer(&self, key: PrewarmKey, snapshot: DesiredProvider) -> bool {
        let mut pending = self.pending.lock().expect("prewarm queue poisoned");
        if pending.len() >= self.capacity && !pending.contains_key(&key) {
            self.dropped();
            return false;
        }
        let epoch = self.next_epoch.fetch_add(1, Ordering::Relaxed);
        self.epochs
            .lock()
            .expect("prewarm epochs poisoned")
            .insert(key.clone(), (epoch, 0));
        pending.insert(key, (snapshot, epoch));
        self.notify.notify_one();
        true
    }
    fn take(&self) -> HashMap<PrewarmKey, (DesiredProvider, u64)> {
        std::mem::take(&mut *self.pending.lock().expect("prewarm queue poisoned"))
    }
    /// The intent reached a final outcome; forget it unless a newer intent replaced it.
    fn settle(&self, key: &PrewarmKey, epoch: u64) {
        let mut epochs = self.epochs.lock().expect("prewarm epochs poisoned");
        if epochs.get(key).is_some_and(|(current, _)| *current == epoch) {
            epochs.remove(key);
        }
    }
    fn busy(
        self: &Arc<Self>,
        key: PrewarmKey,
        snapshot: DesiredProvider,
        epoch: u64,
        stopping: tokio_util::sync::CancellationToken,
    ) {
        let attempt = {
            let mut epochs = self.epochs.lock().expect("prewarm epochs poisoned");
            match epochs.get_mut(&key) {
                Some((current, attempts)) if *current == epoch => {
                    *attempts += 1;
                    *attempts
                }
                // Superseded by a newer intent, which carries its own outcome.
                _ => return,
            }
        };
        let Some(delay) = retry_delay(attempt) else {
            self.settle(&key, epoch);
            self.dropped();
            return;
        };
        let queue = Arc::clone(self);
        tokio::spawn(async move {
            tokio::select! {
                _ = stopping.cancelled() => return,
                _ = tokio::time::sleep(delay) => {}
            }
            queue.requeue(key, snapshot, epoch);
        });
    }
    fn requeue(&self, key: PrewarmKey, snapshot: DesiredProvider, epoch: u64) {
        let mut pending = self.pending.lock().expect("prewarm queue poisoned");
        let current = self
            .epochs
            .lock()
            .expect("prewarm epochs poisoned")
            .get(&key)
            .is_some_and(|(current, _)| *current == epoch);
        if !current || pending.contains_key(&key) {
            return;
        }
        if pending.len() >= self.capacity {
            drop(pending);
            self.settle(&key, epoch);
            self.dropped();
            return;
        }
        pending.insert(key, (snapshot, epoch));
        self.notify.notify_one();
    }
    fn dropped(&self) {
        self.metrics
            .increment(crate::services::provider_runtime::RuntimeCounter::DroppedIntent);
    }
}

async fn snapshots(
    database: &Database,
    provider: Option<(i64, i64)>,
    template: Option<i64>,
) -> Vec<DesiredProvider> {
    database
        .prewarm_provider_rows(provider, template)
        .await
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn busy_retry_never_resurrects_a_snapshot_older_than_the_current_intent() {
        let queue = IntentQueue {
            pending: Mutex::new(HashMap::new()),
            epochs: Mutex::new(HashMap::new()),
            next_epoch: AtomicU64::new(1),
            notify: Notify::new(),
            capacity: 8,
            metrics: Arc::new(Default::default()),
        };
        let key = PrewarmKey::Resource(ResourceKey([7; 32]));
        assert!(queue.offer(key.clone(), snapshot(1, 1)));
        let (_, (_, stale)) = queue.take().into_iter().next().unwrap();
        // The current intent's own retry is re-queued.
        queue.requeue(key.clone(), snapshot(1, 1), stale);
        let (_, (retried, _)) = queue.take().into_iter().next().unwrap();
        assert_eq!(retried.revision, 1);
        // A newer intent is taken and in flight when the older retry fires.
        assert!(queue.offer(key.clone(), snapshot(1, 2)));
        let (_, (current, _)) = queue.take().into_iter().next().unwrap();
        assert_eq!(current.revision, 2);
        queue.requeue(key, snapshot(1, 1), stale);
        assert!(queue.take().is_empty());
    }

    #[test]
    fn busy_intents_back_off_to_a_cap_and_then_give_up() {
        let seconds: Vec<_> = (1..=11)
            .map(|attempt| retry_delay(attempt).map(|delay| delay.as_secs()))
            .collect();
        assert_eq!(
            seconds,
            [
                Some(1),
                Some(2),
                Some(4),
                Some(8),
                Some(16),
                Some(30),
                Some(30),
                Some(30),
                Some(30),
                Some(30),
                None
            ]
        );
    }

    #[test]
    fn pending_snapshots_coalesce_by_resource_and_reject_a_new_resource_at_capacity() {
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let notify = Arc::new(Notify::new());
        let enqueue = |key, snapshot| {
            let mut queue = pending.lock().unwrap();
            if queue.len() >= 2 && !queue.contains_key(&key) {
                return false;
            }
            queue.insert(key, snapshot);
            notify.notify_one();
            true
        };
        let shared = PrewarmKey::Resource(ResourceKey([1; 32]));
        assert!(enqueue(shared.clone(), snapshot(1, 1)));
        assert!(enqueue(
            PrewarmKey::Resource(ResourceKey([2; 32])),
            snapshot(2, 1)
        ));
        assert!(enqueue(shared.clone(), snapshot(1, 9)));
        assert!(!enqueue(
            PrewarmKey::Resource(ResourceKey([3; 32])),
            snapshot(3, 1)
        ));
        let queue = pending.lock().unwrap();
        assert_eq!(queue.len(), 2);
        assert_eq!(queue.get(&shared).unwrap().revision, 9);
    }

    fn snapshot(id: i64, revision: i64) -> DesiredProvider {
        DesiredProvider {
            id,
            key: format!("provider-{id}"),
            kind: "llm".into(),
            adapter: "openai".into(),
            config_json: "{}".into(),
            secret_ref: None,
            revision,
        }
    }

    #[tokio::test]
    async fn obsolete_unbound_and_disabled_intents_never_produce_build_snapshots() {
        let database = Database::connect(&crate::config::DatabaseConfig {
            url: "sqlite::memory:".into(),
            max_connections: 1,
            ..Default::default()
        })
        .await
        .unwrap();
        sqlx::raw_sql(r#"
INSERT INTO agent_templates (key,name,language,prompt,created_at,updated_at) VALUES ('template','Template','vi','prompt',1,1);
INSERT INTO providers (key,name,type,adapter,config_json,revision,created_at,updated_at) VALUES ('llm','LLM','llm','openai','{"base_url":"https://example.test/v1","model":"fixture"}',2,1,1);
INSERT INTO template_provider_bindings (template_id,provider_type,provider_id,created_at,updated_at) VALUES (1,'llm',1,1,1);
"#).execute(database.pool()).await.unwrap();
        assert!(snapshots(&database, Some((1, 1)), None).await.is_empty());
        let current = snapshots(&database, Some((1, 2)), None).await;
        assert_eq!(current.len(), 1);
        assert_eq!(current[0].revision, 2);
        sqlx::query("UPDATE providers SET enabled=0 WHERE id=1")
            .execute(database.pool())
            .await
            .unwrap();
        assert!(snapshots(&database, None, Some(1)).await.is_empty());
        sqlx::raw_sql("UPDATE providers SET enabled=1; DELETE FROM template_provider_bindings;")
            .execute(database.pool())
            .await
            .unwrap();
        assert!(snapshots(&database, Some((1, 2)), None).await.is_empty());
    }
}
