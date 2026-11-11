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
            async move { Some(check < 2) }
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
async fn unknown_ingest_state_is_deferred_until_a_successful_idle_query() {
    let checks = Arc::new(AtomicUsize::new(0));
    let deferred = Arc::new(AtomicUsize::new(0));
    let checks_for_query = Arc::clone(&checks);
    let deferred_for_status = Arc::clone(&deferred);
    let idle = wait_for_ingest_idle(
        move || {
            let check = checks_for_query.fetch_add(1, Ordering::SeqCst);
            async move { if check == 0 { None } else { Some(false) } }
        },
        CancellationToken::new(),
        Duration::from_millis(5),
        Duration::from_secs(1),
        move |_| {
            deferred_for_status.fetch_add(1, Ordering::SeqCst);
        },
    )
    .await;
    assert!(idle);
    assert_eq!(checks.load(Ordering::SeqCst), 2);
    assert_eq!(deferred.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn sync_times_out_waiting_for_knowledge_writer_and_publishes_timeout_status() {
    if !git_available() {
        return;
    }
    let root = std::env::temp_dir().join(format!("wiki-sync-gate-{}", uuid::Uuid::new_v4()));
    let remote = std::env::temp_dir().join(format!("wiki-sync-gate-bare-{}", uuid::Uuid::new_v4()));
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
    config.command_timeout = Duration::from_millis(40);
    let sync = Arc::new(VaultSync::configure(Arc::clone(&vault), config).unwrap());
    let gate = Arc::new(KnowledgeWriteGate::default());
    let service = WikiSyncService::new(
        vault,
        sync,
        Arc::clone(&gate),
        None,
        CancellationToken::new(),
    );
    let guard = gate.write().await;
    let started = Instant::now();
    let error = service.sync_now().await.unwrap_err();
    assert!(started.elapsed() < Duration::from_secs(1));
    assert!(matches!(error, SyncError::Timeout { .. }));
    assert!(matches!(
        service.status(),
        SyncStatus::Error {
            kind: wiki_core::SyncErrorKind::Timeout,
            ..
        }
    ));
    drop(guard);
    let _ = std::fs::remove_dir_all(root);
    let _ = std::fs::remove_dir_all(remote);
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
