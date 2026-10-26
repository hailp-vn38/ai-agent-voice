//! Bounded, coalesced desired-state hints. Mutations enqueue only after committing SQLite.
use crate::{
    database::{Database, DesiredProvider},
    services::provider_runtime::{ProviderRuntimeManager, ProviderVersion, ResourceKey},
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, Weak},
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
    pending: Arc<Mutex<HashMap<PrewarmKey, DesiredProvider>>>,
    notify: Arc<Notify>,
    capacity: usize,
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
        let service = Arc::new(Self {
            database,
            config,
            manager: Arc::downgrade(manager),
            pending: Arc::new(Mutex::new(HashMap::new())),
            notify: Arc::new(Notify::new()),
            capacity: (manager.prewarm_capacity() * 4).max(8),
            metrics: manager.metrics().clone(),
        });
        let metrics = service.metrics.clone();
        let pending = service.pending.clone();
        let notify = service.notify.clone();
        let manager = service.manager.clone();
        let stopping = manager.upgrade()?.stopping_token();
        runtime.spawn(async move {
            loop {
                tokio::select! { _ = stopping.cancelled() => break, _ = notify.notified() => {} }
                // Coalesce rapid edits without holding a DB transaction or delaying their HTTP response.
                tokio::select! { _ = stopping.cancelled() => break, _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {} }
                let snapshots = std::mem::take(&mut *pending.lock().expect("prewarm queue poisoned"));
                let mut batch_leases = std::collections::VecDeque::with_capacity(4);
                for snapshot in snapshots.into_values() {
                    if stopping.is_cancelled() { break; }
                    let Some(manager) = manager.upgrade() else { return };
                    // Hold at most one template's 4 active bindings simultaneously so cross-template
                    // batches do not exhaust max_resources while earlier bindings in the same
                    // template stay pinned until the template batch finishes.
                    while batch_leases.len() >= 4 {
                        batch_leases.pop_front();
                    }
                    // No inference operation or inference capacity is consumed here. Busy
                    // speculation is dropped; explicit acquisition remains available.
                    match manager.acquire_speculative(snapshot).await {
                        Ok(lease) => batch_leases.push_back(lease),
                        Err(crate::services::provider_runtime::RuntimeError::Busy) => {
                            metrics.increment(crate::services::provider_runtime::RuntimeCounter::DroppedIntent);
                        }
                        Err(_) => {}
                    }
                }
                drop(batch_leases);
            }
        });
        Some(service)
    }
    fn enqueue(&self, key: PrewarmKey, snapshot: DesiredProvider) -> bool {
        let mut pending = self.pending.lock().expect("prewarm queue poisoned");
        if pending.len() >= self.capacity && !pending.contains_key(&key) {
            self.metrics
                .increment(crate::services::provider_runtime::RuntimeCounter::DroppedIntent);
            return false;
        }
        pending.insert(key, snapshot);
        self.notify.notify_one();
        true
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
