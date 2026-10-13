use super::*;

#[tokio::test]
async fn database_and_config_providers_with_same_key_are_distinct_immutable_choices() {
    let (db, config, runtime) = harness().await;
    let inserted=sqlx::query("INSERT INTO providers(key,name,type,adapter,config_json,created_at,updated_at) VALUES('planner','Database planner','llm','openai','{}',0,0)")
        .execute(db.pool()).await.unwrap().last_insert_rowid();
    let revision = runtime.engine.settings().await.unwrap().revision;
    runtime
        .engine
        .update_settings(
            BackgroundSettingsUpdate {
                enabled: true,
                orchestrator_llm_provider: Some("planner".into()),
                worker_llm_provider: Some("config:llm:planner".into()),
            },
            revision,
        )
        .await
        .unwrap();
    let job = runtime
        .engine
        .submit(request("Explicit pair"), &JobRequester::Admin, None)
        .await
        .unwrap()
        .job;
    assert_eq!(job.snapshot.orchestrator.reference, "planner");
    assert_eq!(job.snapshot.orchestrator.incarnation, inserted);
    assert_eq!(job.snapshot.worker.reference, "config:llm:planner");
    assert_eq!(job.snapshot.worker.incarnation, 0);
    runtime.shutdown().await;
    sqlx::query("DELETE FROM providers WHERE key='planner'")
        .execute(db.pool())
        .await
        .unwrap();
    sqlx::query("INSERT INTO providers(key,name,type,adapter,config_json,created_at,updated_at) VALUES('planner','Replacement','llm','openai','{}',0,0)")
        .execute(db.pool()).await.unwrap();
    let restarted = BackgroundRuntime::foundation_harness(db, config)
        .await
        .unwrap();
    let historical = restarted.engine.get(&job.id).await.unwrap();
    assert_eq!(historical.snapshot.orchestrator.incarnation, inserted);
    assert_eq!(
        historical.error_code.as_deref(),
        Some("configuration_changed")
    );
    restarted.shutdown().await;
}

#[tokio::test]
async fn concurrent_idempotent_requests_commit_exactly_one_job() {
    let (_, _, runtime) = harness().await;
    let mut requests = Vec::new();
    for _ in 0..8 {
        let engine = runtime.engine.clone();
        requests.push(tokio::spawn(async move {
            engine
                .submit(
                    request("Same intent"),
                    &JobRequester::Admin,
                    Some("concurrent-once"),
                )
                .await
                .unwrap()
        }));
    }
    let mut id = None;
    let mut first = 0;
    for request in requests {
        let receipt = request.await.unwrap();
        if !receipt.replayed {
            first += 1;
        }
        if let Some(id) = &id {
            assert_eq!(&receipt.job.id, id);
        } else {
            id = Some(receipt.job.id);
        }
    }
    assert_eq!(first, 1);
    assert_eq!(runtime.engine.list(1, 30, None, None).await.unwrap().1, 1);
    runtime.shutdown().await;
}

#[tokio::test]
async fn background_stream_requires_finished_even_after_valid_json() {
    use crate::providers::LlmEvent;
    let json = r#"{"summary":"valid but unconfirmed"}"#;
    let stream = Box::pin(futures_util::stream::iter([Ok(LlmEvent::TextDelta(
        json.into(),
    ))]));
    assert!(matches!(
        crate::providers::llm::complete_background_chat(stream).await,
        Err(crate::providers::LlmError::Failed)
    ));
    let stream = Box::pin(futures_util::stream::iter([
        Ok(LlmEvent::TextDelta(json.into())),
        Ok(LlmEvent::Finished),
    ]));
    assert_eq!(
        crate::providers::llm::complete_background_chat(stream).await.unwrap(),
        json
    );
}
