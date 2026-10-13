
#[tokio::test]
async fn p4_acceptance_retention_preserves_inflight_artifacts() {
    let (db, _, runtime) = harness().await;
    let old = runtime.engine.submit(request("expired"), &JobRequester::Admin, None).await.unwrap().job;
    let active = runtime.engine.submit(request("publishing"), &JobRequester::Admin, None).await.unwrap().job;
    runtime.shutdown().await;
    let now = crate::database::unix_seconds().unwrap();
    sqlx::query("UPDATE jobs SET status='failed',updated_at=? WHERE id=?")
        .bind(now - 31 * 24 * 60 * 60).bind(&old.id).execute(db.pool()).await.unwrap();
    sqlx::query("UPDATE jobs SET status='running',updated_at=? WHERE id=?")
        .bind(now).bind(&active.id).execute(db.pool()).await.unwrap();
    let dir = crate::services::background::artifacts::root(&db).unwrap().join(&active.id);
    tokio::fs::create_dir_all(&dir).await.unwrap();
    // Pause publication after rename and before SQLite metadata commit.
    let inflight = dir.join(uuid::Uuid::new_v4().to_string());
    tokio::fs::write(&inflight, b"pending commit").await.unwrap();
    assert_eq!(crate::services::background::artifacts::expire_terminal(&db, now).await.unwrap(), 1);
    assert!(inflight.exists(), "retention deleted an artifact of a running Job before metadata commit");
}
