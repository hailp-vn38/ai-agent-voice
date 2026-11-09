use crate::app::{AppState, KnowledgeWriteGate};
use orchestrator_flow_rs::{TaskFilter, TaskManager};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime},
};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};
use wiki_core::{DeferReason, DisabledReason, SyncError, SyncReport, SyncStatus, Vault, VaultSync};

const INGEST_RETRY: Duration = Duration::from_secs(30);
const INGEST_MAX_WAIT: Duration = Duration::from_secs(10 * 60);
const SCHEDULER_POLL: Duration = Duration::from_millis(250);

pub(crate) struct WikiSyncService {
    vault: Arc<Vault>,
    sync: Arc<VaultSync>,
    write_gate: Arc<KnowledgeWriteGate>,
    task_manager: Option<Arc<TaskManager>>,
    stopping: CancellationToken,
    status: Mutex<SyncStatus>,
    status_events: broadcast::Sender<SyncStatus>,
    sync_in_progress: AtomicBool,
}

impl WikiSyncService {
    pub(crate) fn new(
        vault: Arc<Vault>,
        sync: Arc<VaultSync>,
        write_gate: Arc<KnowledgeWriteGate>,
        task_manager: Option<Arc<TaskManager>>,
        stopping: CancellationToken,
    ) -> Arc<Self> {
        let (status_events, _) = broadcast::channel(32);
        let initial_status = safe_status(sync.status());
        Arc::new(Self {
            vault,
            status: Mutex::new(initial_status),
            sync,
            write_gate,
            task_manager,
            stopping,
            status_events,
            sync_in_progress: AtomicBool::new(false),
        })
    }

    // Admin HTTP/SSE adapters consume these in the follow-on sync-status ticket.
    #[allow(dead_code)]
    pub(crate) fn status(&self) -> SyncStatus {
        self.status
            .lock()
            .map(|status| status.clone())
            .unwrap_or_else(|_| safe_status(self.sync.status()))
    }

    #[allow(dead_code)]
    pub(crate) fn subscribe(&self) -> broadcast::Receiver<SyncStatus> {
        self.status_events.subscribe()
    }

    pub(crate) fn publish(&self, status: SyncStatus) {
        let status = safe_status(status);
        if let Ok(mut current) = self.status.lock() {
            *current = status.clone();
        }
        let _ = self.status_events.send(status);
    }

    pub(crate) async fn sync_now(self: &Arc<Self>) -> Result<SyncReport, SyncError> {
        if self
            .sync_in_progress
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(SyncError::Git("sync_in_progress".into()));
        }
        struct RunGuard<'a>(&'a AtomicBool);
        impl Drop for RunGuard<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::Release);
            }
        }
        let _run_guard = RunGuard(&self.sync_in_progress);
        let service = Arc::clone(self);
        let idle = wait_for_ingest_idle(
            || service.ingest_job_running(),
            service.stopping.clone(),
            INGEST_RETRY,
            INGEST_MAX_WAIT,
            |retry_at| {
                service.publish(SyncStatus::Deferred {
                    reason: DeferReason::IngestRunning,
                    retry_at,
                });
            },
        )
        .await;
        if !idle {
            return Err(SyncError::Git("sync cancelled during shutdown".into()));
        }

        self.publish(self.sync.status());
        let _write_guard = self.write_gate.write().await;
        let sync = Arc::clone(&self.sync);
        tokio::task::spawn_blocking(move || sync.sync_now())
            .await
            .map_err(|error| SyncError::Git(format!("sync worker failed: {error}")))?
    }

    pub(crate) fn is_sync_in_progress(&self) -> bool {
        self.sync_in_progress.load(Ordering::Acquire)
    }

    async fn ingest_job_running(&self) -> bool {
        let Some(task_manager) = &self.task_manager else {
            return false;
        };
        let filter = TaskFilter {
            agent_id: Some("wiki_root".into()),
            limit: Some(100),
            ..Default::default()
        };
        match task_manager.list_all_tasks(&filter).await {
            Ok(page) => page
                .items
                .into_iter()
                .any(|task| !task.status.is_terminal()),
            Err(error) => {
                warn!(error = %error, "could not inspect active Wiki ingest tasks before sync");
                false
            }
        }
    }

    fn forward_status_events(self: &Arc<Self>) {
        let mut source = self.sync.subscribe();
        let service = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = service.stopping.cancelled() => break,
                    event = source.recv() => match event {
                        Ok(status) => service.publish(status),
                        Err(broadcast::error::RecvError::Lagged(count)) => {
                            warn!(count, "Wiki sync status subscriber lagged");
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }
            }
        });
    }

    fn start_scheduler(self: &Arc<Self>, debounce: Duration, interval: Duration) {
        self.forward_status_events();
        if !self.vault.is_writable()
            || matches!(
                self.sync.status(),
                SyncStatus::Disabled {
                    reason: DisabledReason::NoGit
                        | DisabledReason::ReadOnly
                        | DisabledReason::NotConfigured
                }
            )
        {
            info!("Wiki sync scheduler disabled because this process cannot sync the vault");
            return;
        }

        let service = Arc::clone(self);
        let write_gate = Arc::clone(&self.write_gate);
        tokio::spawn(async move {
            if service.stopping.is_cancelled() {
                return;
            }
            if let Err(error) = service.sync_now().await {
                warn!(kind = ?error.kind(), "initial Wiki vault sync failed");
            }
            let mut synced_write = write_gate.last_write();
            let mut next_periodic = Instant::now() + interval;
            loop {
                tokio::select! {
                    _ = service.stopping.cancelled() => break,
                    _ = tokio::time::sleep(SCHEDULER_POLL) => {}
                }
                let last_write = write_gate.last_write();
                let write_due = last_write.is_some_and(|written| {
                    Some(written) != synced_write && written.elapsed() >= debounce
                });
                let periodic_due = Instant::now() >= next_periodic;
                if !write_due && !periodic_due {
                    continue;
                }
                if let Err(error) = service.sync_now().await {
                    warn!(kind = ?error.kind(), "scheduled Wiki vault sync failed");
                }
                synced_write = write_gate.last_write();
                next_periodic = Instant::now() + interval;
            }
        });
    }
}

fn safe_status(status: SyncStatus) -> SyncStatus {
    match status {
        SyncStatus::Error { kind, .. } => SyncStatus::Error {
            kind,
            message: match kind {
                wiki_core::SyncErrorKind::Auth => "Authentication failed".into(),
                wiki_core::SyncErrorKind::Network => "Network operation failed".into(),
                wiki_core::SyncErrorKind::Timeout => "Git operation timed out".into(),
                wiki_core::SyncErrorKind::UnsafeRemote => "Remote URL is unsafe".into(),
                wiki_core::SyncErrorKind::ReadOnly => "Vault is read-only".into(),
                wiki_core::SyncErrorKind::GitMissing => "Git is unavailable".into(),
                wiki_core::SyncErrorKind::FileTooLarge => {
                    "A vault file exceeds the configured size limit".into()
                }
                wiki_core::SyncErrorKind::Vault => "Vault operation failed".into(),
                wiki_core::SyncErrorKind::Git => "Git operation failed".into(),
            },
        },
        status => status,
    }
}

async fn wait_for_ingest_idle<F, Fut, P>(
    mut is_running: F,
    stopping: CancellationToken,
    retry: Duration,
    max_wait: Duration,
    mut deferred: P,
) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
    P: FnMut(SystemTime),
{
    let started = Instant::now();
    loop {
        if !is_running().await {
            return true;
        }
        if started.elapsed() >= max_wait {
            warn!("Wiki ingest remained active for 10 minutes; syncing anyway");
            return true;
        }
        deferred(SystemTime::now() + retry);
        tokio::select! {
            _ = stopping.cancelled() => return false,
            _ = tokio::time::sleep(retry.min(max_wait.saturating_sub(started.elapsed()))) => {}
        }
    }
}

pub(crate) fn start_if_enabled(state: &AppState) {
    if !state.config.wiki.sync.enabled
        || state
            .wiki_sync_scheduler_started
            .swap(true, Ordering::AcqRel)
    {
        return;
    }
    let state = state.clone();
    tokio::spawn(async move {
        let Some(engine) = state.wiki_engine.as_deref() else {
            warn!("Wiki sync enabled but the Wiki engine is unavailable");
            return;
        };
        let Some(vault) = state.get_or_init_wiki_vault(engine) else {
            warn!("Wiki sync enabled but the Vault could not be opened");
            return;
        };
        let config = state.config.wiki.sync.to_core_config();
        let sync_vault = Arc::clone(&vault);
        let sync =
            match tokio::task::spawn_blocking(move || VaultSync::configure(sync_vault, config))
                .await
            {
                Ok(Ok(sync)) => Arc::new(sync),
                Ok(Err(error)) => {
                    warn!(kind = ?error.kind(), "Wiki sync configuration failed");
                    return;
                }
                Err(error) => {
                    warn!(error = %error, "Wiki sync configuration worker failed");
                    return;
                }
            };
        let service = WikiSyncService::new(
            vault,
            sync,
            Arc::clone(&state.knowledge_write_gate),
            state.task_manager.clone(),
            state.lifecycle.stopping().clone(),
        );
        let _ = state.wiki_sync_service.set(Arc::clone(&service));
        let config = &state.config.wiki.sync;
        service.start_scheduler(
            Duration::from_secs(config.debounce_sec),
            Duration::from_secs(config.interval_min.saturating_mul(60)),
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, extract::State, http::StatusCode, middleware, routing::put};
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn published_sync_errors_do_not_include_git_stderr() {
        let status = safe_status(SyncStatus::Error {
            kind: wiki_core::SyncErrorKind::Auth,
            message: "fatal: Authentication failed for secret-token".into(),
        });
        assert!(
            matches!(status, SyncStatus::Error { message, .. } if message == "Authentication failed")
        );
    }

    #[tokio::test]
    async fn active_ingest_defers_then_sync_can_continue_when_it_finishes() {
        let checks = Arc::new(AtomicUsize::new(0));
        let deferred = Arc::new(AtomicUsize::new(0));
        let checks_for_query = Arc::clone(&checks);
        let deferred_for_status = Arc::clone(&deferred);
        let idle = wait_for_ingest_idle(
            move || {
                let check = checks_for_query.fetch_add(1, Ordering::SeqCst);
                async move { check < 2 }
            },
            CancellationToken::new(),
            Duration::from_millis(5),
            Duration::from_secs(1),
            move |retry_at| {
                assert!(retry_at > SystemTime::now());
                deferred_for_status.fetch_add(1, Ordering::SeqCst);
            },
        )
        .await;
        assert!(idle);
        assert_eq!(checks.load(Ordering::SeqCst), 3);
        assert_eq!(deferred.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn knowledge_route_write_is_committed_after_debounce() {
        if !git_available() {
            eprintln!("SKIPPED: git is not installed");
            return;
        }
        let root = std::env::temp_dir().join(format!("wiki-sync-route-{}", uuid::Uuid::new_v4()));
        let remote = std::env::temp_dir().join(format!("wiki-sync-bare-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&remote).unwrap();
        let init = std::process::Command::new("git")
            .args(["init", "--bare", "--initial-branch=main"])
            .arg(&remote)
            .output()
            .unwrap();
        assert!(init.status.success());

        let vault = Arc::new(Vault::open(wiki_core::VaultConfig::new(&root)).unwrap());
        let mut config = wiki_core::SyncConfig::default();
        config.remote_url = Some(remote.to_string_lossy().into_owned());
        config.author = Some(wiki_core::GitAuthor {
            name: "Test Server".into(),
            email: "test@example.invalid".into(),
        });
        let sync = Arc::new(VaultSync::configure(Arc::clone(&vault), config).unwrap());
        let gate = Arc::new(KnowledgeWriteGate::default());
        let stopping = CancellationToken::new();
        let service = WikiSyncService::new(
            Arc::clone(&vault),
            sync,
            Arc::clone(&gate),
            None,
            stopping.clone(),
        );
        service.start_scheduler(Duration::from_millis(30), Duration::from_secs(3_600));

        async fn write_page(State(root): State<std::path::PathBuf>) -> StatusCode {
            std::fs::write(
                root.join("wiki/debounced.md"),
                "# Committed after debounce\n",
            )
            .unwrap();
            StatusCode::OK
        }
        let app = Router::new()
            .route("/knowledge/pages/debounced", put(write_page))
            .route_layer(middleware::from_fn_with_state(
                Arc::clone(&gate),
                crate::app::gate_knowledge_writes,
            ))
            .with_state(root.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let response = reqwest::Client::new()
            .put(format!("http://{address}/knowledge/pages/debounced"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                let output = std::process::Command::new("git")
                    .arg("--git-dir")
                    .arg(&remote)
                    .args(["show", "main:wiki/debounced.md"])
                    .output()
                    .unwrap();
                if output.status.success() {
                    assert_eq!(
                        String::from_utf8_lossy(&output.stdout),
                        "# Committed after debounce\n"
                    );
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("debounced route write reaches the bare remote");
        stopping.cancel();
        server.abort();
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(remote);
    }

    #[tokio::test]
    async fn reader_vault_reports_read_only_and_does_not_start_scheduler() {
        if !git_available() {
            eprintln!("SKIPPED: git is not installed");
            return;
        }
        let root = std::env::temp_dir().join(format!("wiki-sync-reader-{}", uuid::Uuid::new_v4()));
        let remote =
            std::env::temp_dir().join(format!("wiki-sync-reader-bare-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&remote).unwrap();
        let init = std::process::Command::new("git")
            .args(["init", "--bare", "--initial-branch=main"])
            .arg(&remote)
            .output()
            .unwrap();
        assert!(init.status.success());
        let _writer = Arc::new(Vault::open(wiki_core::VaultConfig::new(&root)).unwrap());
        let reader = Arc::new(Vault::open(wiki_core::VaultConfig::new(&root)).unwrap());
        assert!(!reader.is_writable());
        let mut config = wiki_core::SyncConfig::default();
        config.remote_url = Some(remote.to_string_lossy().into_owned());
        let sync = Arc::new(VaultSync::configure(Arc::clone(&reader), config).unwrap());
        assert!(matches!(
            sync.status(),
            SyncStatus::Disabled {
                reason: DisabledReason::ReadOnly
            }
        ));
        let gate = Arc::new(KnowledgeWriteGate::default());
        let stopping = CancellationToken::new();
        let service = WikiSyncService::new(reader, sync, gate, None, stopping.clone());
        service.start_scheduler(Duration::from_millis(10), Duration::from_millis(20));
        tokio::time::sleep(Duration::from_millis(80)).await;
        assert!(matches!(
            service.status(),
            SyncStatus::Disabled {
                reason: DisabledReason::ReadOnly
            }
        ));
        stopping.cancel();
        let _ = std::fs::remove_dir_all(root);
        let _ = std::fs::remove_dir_all(remote);
    }

    fn git_available() -> bool {
        std::process::Command::new("git")
            .arg("--version")
            .output()
            .is_ok_and(|result| result.status.success())
    }
}
