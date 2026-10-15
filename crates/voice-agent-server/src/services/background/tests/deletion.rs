use super::*;

#[tokio::test]
async fn delete_accepts_all_terminal_statuses_and_preserves_successful_retry() {
    let (db, _, runtime) = harness().await;
    for status in ["failed", "succeeded", "partial_success", "cancelled"] {
        let job = runtime
            .engine
            .submit(request("Delete finished Job"), &JobRequester::Admin, None)
            .await
            .unwrap()
            .job;
        sqlx::query("UPDATE jobs SET status=?,error_code=? WHERE id=?")
            .bind(status)
            .bind((status == "failed").then_some("llm_execution_failed"))
            .bind(&job.id)
            .execute(db.pool())
            .await
            .unwrap();
        if status == "failed" {
            let retry = runtime
                .engine
                .retry(&job.id, true, Some("retained-retry"))
                .await
                .unwrap()
                .job;
            let result = json!({"summary":"Keep this successful result"});
            sqlx::query("UPDATE jobs SET status='succeeded',result_json=? WHERE id=?")
                .bind(result.to_string())
                .bind(&retry.id)
                .execute(db.pool())
                .await
                .unwrap();
            runtime.engine.delete(&job.id).await.unwrap();
            let retained = runtime.engine.get(&retry.id).await.unwrap();
            assert_eq!(retained.status, "succeeded");
            assert_eq!(retained.result, Some(result));
            assert_eq!(retained.retry_of_job_id, None);
            assert_eq!(
                runtime
                    .engine
                    .submit(
                        request("Delete finished Job"),
                        &JobRequester::Admin,
                        Some("retained-retry")
                    )
                    .await
                    .unwrap_err(),
                JobError::IdempotencyConflict,
            );
            // Detaching a deleted parent must not permit editing terminal results.
            assert!(
                sqlx::query("UPDATE jobs SET result_json='{}' WHERE id=?")
                    .bind(&retry.id)
                    .execute(db.pool())
                    .await
                    .is_err()
            );
            assert!(
                sqlx::query("UPDATE jobs SET retry_of_job_id=? WHERE id=?")
                    .bind(&retry.id)
                    .bind(&retry.id)
                    .execute(db.pool())
                    .await
                    .is_err()
            );
        } else {
            runtime.engine.delete(&job.id).await.unwrap();
        }
        assert!(matches!(
            runtime.engine.get(&job.id).await,
            Err(JobError::NotFound)
        ));
    }
    runtime.shutdown().await;
}

#[tokio::test]
async fn delete_reaps_only_owned_external_attempts_and_preserves_data_on_cleanup_failure() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let socket = std::env::temp_dir().join(format!("delete-{}.sock", uuid::Uuid::new_v4()));
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let mut config = config();
    config.background.plugin_runner_socket = Some(socket.clone());
    let config = Arc::new(config);
    let db = Arc::new(Database::connect(&config.database).await.unwrap());
    let runtime = BackgroundRuntime::foundation_harness(db.clone(), config)
        .await
        .unwrap();
    enable(&runtime.engine).await;
    let job = runtime
        .engine
        .submit(
            request("Delete orphaned worker"),
            &JobRequester::Admin,
            None,
        )
        .await
        .unwrap()
        .job;
    let attempt = uuid::Uuid::new_v4().to_string();
    let other = runtime
        .engine
        .submit(request("Keep another worker"), &JobRequester::Admin, None)
        .await
        .unwrap()
        .job;
    runtime.engine.cancel(&job.id).await.unwrap();
    sqlx::query("UPDATE jobs SET status='running' WHERE id=?")
        .bind(&other.id)
        .execute(db.pool())
        .await
        .unwrap();
    sqlx::query("INSERT INTO job_steps(job_id,step_key,plan_revision,status,installation_digest,attempt_id,attempt_state,worker_instance_id,created_at,updated_at) VALUES(?,'external',1,'failed','external-install',?,'unknown_outcome','owned-worker',0,0)")
        .bind(&job.id).bind(&attempt).execute(db.pool()).await.unwrap();
    sqlx::query("INSERT INTO job_steps(job_id,step_key,plan_revision,status,installation_digest,attempt_id,attempt_state,worker_instance_id,created_at,updated_at) VALUES(?,'external',1,'running','external-install',?,'started','other-worker',0,0)")
        .bind(&other.id).bind(uuid::Uuid::new_v4().to_string()).execute(db.pool()).await.unwrap();
    let runner = tokio::spawn(async move {
        for (action, state) in [
            ("Cancel", "unavailable"),
            ("Cancel", "cancelling"),
            ("GetAttempt", "terminal"),
        ] {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            stream.read_to_end(&mut bytes).await.unwrap();
            let input: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(input["action"], action);
            assert_eq!(input["attempt_id"], attempt);
            let reply = json!({"ok":true,"value":{"state":state}});
            stream
                .write_all(format!("{reply}\n").as_bytes())
                .await
                .unwrap();
        }
    });
    assert_eq!(
        runtime.engine.delete(&job.id).await.unwrap_err(),
        JobError::DeleteConflict
    );
    assert_eq!(runtime.engine.get(&job.id).await.unwrap().steps.len(), 1);
    runtime.engine.delete(&job.id).await.unwrap();
    assert!(matches!(
        runtime.engine.get(&job.id).await,
        Err(JobError::NotFound)
    ));
    assert_eq!(
        runtime.engine.get(&other.id).await.unwrap().steps[0].status,
        "running"
    );
    runner.await.unwrap();
    tokio::fs::remove_file(socket).await.unwrap();
    runtime.shutdown().await;
}
