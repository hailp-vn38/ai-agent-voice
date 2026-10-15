use super::protocol::parse_response;
use serde_json::json;
use std::io::Read;

#[tokio::test]
async fn package_failure_persists_error_and_cannot_poison_new_installation() {
    let (db, _, runtime) = crate::services::background::tests::harness().await;
    let bytes = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../plugins/fixture-research-1.0.0.zip"
    ));
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut manifest = String::new();
    zip.by_name("plugin.toml")
        .unwrap()
        .read_to_string(&mut manifest)
        .unwrap();
    let manifest =
        serde_json::to_string(&toml::from_str::<toml::Value>(&manifest).unwrap()).unwrap();
    let choice = super::FrozenChoice {
        plugin_id: "fixture-research".into(),
        version: "1.0.0".into(),
        package_digest: "a".repeat(64),
        installation_digest: format!("sha256:{}", "b".repeat(64)),
        operation: "collect".into(),
    };
    sqlx::query("INSERT INTO plugin_installations(plugin_id,version,package_digest,installation_digest,status,revision,operations_json,created_at,updated_at) VALUES(?,?,?,?,'ready',1,?,0,0)")
        .bind(&choice.plugin_id).bind(&choice.version).bind(&choice.package_digest)
        .bind(&choice.installation_digest).bind(manifest).execute(db.pool()).await.unwrap();
    runtime
        .engine
        .plugins
        .fail_package(&choice, "worker_protocol_error")
        .await
        .unwrap();
    let failed = runtime.engine.plugins.get(&choice.plugin_id).await.unwrap();
    assert_eq!(failed.status, "error");
    assert_eq!(
        failed.last_error_code.as_deref(),
        Some("worker_protocol_error")
    );
    assert_eq!(failed.revision, 2);
    sqlx::query("UPDATE plugin_installations SET status='ready',installation_digest=?,revision=revision+1 WHERE plugin_id=?")
        .bind(format!("sha256:{}", "d".repeat(64))).bind(&choice.plugin_id).execute(db.pool()).await.unwrap();
    runtime
        .engine
        .plugins
        .fail_package(&choice, "plugin_runtime_error")
        .await
        .unwrap();
    assert_eq!(
        runtime
            .engine
            .plugins
            .get(&choice.plugin_id)
            .await
            .unwrap()
            .status,
        "ready"
    );
    runtime.shutdown().await;
}

#[tokio::test]
async fn startup_reaper_removes_only_owned_uuid_workspaces() {
    let (db, config, runtime) = crate::services::background::tests::harness().await;
    runtime.shutdown().await;
    let mut store = super::PluginStore::new(db, config.background.clone());
    store.root = std::env::temp_dir().join(format!("plugin-cleanup-{}", uuid::Uuid::new_v4()));
    let job = uuid::Uuid::new_v4().to_string();
    let attempt = uuid::Uuid::new_v4().to_string();
    let stale = store.root.join("workspaces").join(&job).join(&attempt);
    tokio::fs::create_dir_all(stale.join("out")).await.unwrap();
    tokio::fs::write(stale.join("out").join("result"), b"old")
        .await
        .unwrap();
    for _ in 0..1000 {
        tokio::fs::create_dir_all(
            store
                .root
                .join("workspaces")
                .join(&job)
                .join(uuid::Uuid::new_v4().to_string()),
        )
        .await
        .unwrap();
    }
    store.drain_workspaces().await.unwrap();
    assert!(!stale.exists());
    let unexpected = store.root.join("workspaces").join("not-owned");
    tokio::fs::create_dir_all(&unexpected).await.unwrap();
    assert_eq!(
        store.drain_workspaces().await.err(),
        Some(super::PluginError::StorageUnavailable)
    );
    assert!(unexpected.exists());
    tokio::fs::remove_dir_all(unexpected).await.unwrap();
    tokio::fs::remove_dir_all(store.root).await.unwrap();
}

#[tokio::test]
async fn worker_saturation_waits_for_release_and_expires_at_job_deadline() {
    let slots = tokio::sync::Semaphore::new(1);
    let held = slots.acquire().await.unwrap();
    let deadline = crate::database::unix_seconds().unwrap() + 2;
    let waiting = super::dispatch::acquire_worker_until(&slots, deadline);
    tokio::pin!(waiting);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), &mut waiting)
            .await
            .is_err()
    );
    drop(held);
    assert!(waiting.await.is_ok());

    let held = slots.acquire().await.unwrap();
    let expired =
        super::dispatch::acquire_worker_until(&slots, crate::database::unix_seconds().unwrap() - 1)
            .await;
    assert_eq!(
        expired.err(),
        Some(super::PluginError::JobRejected(
            crate::services::background::JobError::DeadlineExceeded
        ))
    );
    drop(held);
}

#[test]
fn worker_timeout_distinguishes_job_deadline_from_step_limit() {
    let now = crate::database::unix_seconds().unwrap();
    assert_eq!(
        super::dispatch::classify_worker_timeout(now - 1),
        super::PluginError::WorkerDeadlineExceeded
    );
    assert_eq!(
        super::dispatch::classify_worker_timeout(now + 30),
        super::PluginError::WorkerTimeout
    );
}

#[test]
fn worker_response_requires_one_bounded_matching_schema_line() {
    let schema = json!({"type":"object","required":["message"],"properties":{"message":{"type":"string"}},"additionalProperties":false});
    let valid = b"{\"schema_version\":1,\"request_id\":\"attempt-1\",\"ok\":true,\"result\":{\"message\":\"hello\"},\"artifacts\":[]}\n";
    assert!(parse_response(valid, "attempt-1", &schema).is_ok());
    assert!(parse_response(valid, "attempt-2", &schema).is_err());
    assert!(
        parse_response(
            &[valid.as_slice(), valid.as_slice()].concat(),
            "attempt-1",
            &schema
        )
        .is_err()
    );
    let wrong = b"{\"schema_version\":1,\"request_id\":\"attempt-1\",\"ok\":true,\"result\":{\"message\":4},\"artifacts\":[]}\n";
    assert!(parse_response(wrong, "attempt-1", &schema).is_err());
    assert!(parse_response(&vec![b'x'; 65537], "attempt-1", &schema).is_err());
}

#[tokio::test]
async fn installation_publication_failure_preserves_previous_revision() {
    use super::PluginError;
    let (db, _, runtime) = crate::services::background::tests::harness().await;
    let digest = "a".repeat(64);
    let installation = format!("sha256:{}", "b".repeat(64));
    let first = db
        .publish_plugin_installation(
            "fixture-research",
            "1.0.0",
            &digest,
            &installation,
            "{}",
            || async { Ok(()) },
        )
        .await
        .unwrap();
    assert_eq!(first.status, "disabled");
    assert_eq!(first.revision, 1);
    let replacement = db
        .publish_plugin_installation(
            "fixture-research",
            "2.0.0",
            &digest,
            &installation,
            "{}",
            || async { Err(PluginError::StorageUnavailable) },
        )
        .await;
    assert_eq!(replacement.err(), Some(PluginError::StorageUnavailable));
    let retained = db.plugin_installation("fixture-research").await.unwrap();
    assert_eq!(retained.version, "1.0.0");
    assert_eq!(retained.revision, 1);
    assert_eq!(
        db.activate_plugin_installation("fixture-research", 2)
            .await
            .err(),
        Some(PluginError::RevisionConflict)
    );
    let activated = db
        .activate_plugin_installation("fixture-research", 1)
        .await
        .unwrap();
    assert_eq!(activated.status, "ready");
    assert_eq!(activated.revision, 2);
    runtime.shutdown().await;
}

#[tokio::test]
async fn worker_outputs_accept_binary_media_without_loading_into_memory() {
    let out = std::env::temp_dir().join(format!("worker-artifacts-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&out).await.unwrap();
    tokio::fs::write(out.join("movie.mp4"), vec![7u8; 2048])
        .await
        .unwrap();
    let artifacts = super::dispatch::read_artifacts(
        &out,
        vec![super::protocol::ArtifactRef {
            path: "movie.mp4".into(),
            media_type: "video/mp4".into(),
            size_bytes: Some(2048),
            sha256: None,
        }],
        4096,
    )
    .await
    .unwrap();
    assert_eq!(artifacts.len(), 1);
    assert_eq!(artifacts[0].filename, "movie.mp4");
    assert_eq!(artifacts[0].source_path, out.join("movie.mp4"));
    tokio::fs::remove_dir_all(out).await.unwrap();
}
