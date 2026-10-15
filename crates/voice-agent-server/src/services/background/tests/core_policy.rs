use super::*;
use crate::services::background::plugins::decode_choice;

#[tokio::test]
async fn core_policy_defaults_and_persists_without_installations() {
    let (db, config, runtime) = harness().await;
    let rows = db.core_plugin_settings().await.unwrap();
    assert_eq!(rows.len(), 4);
    assert!(rows.iter().all(|row| row.enabled == 1 && row.revision == 1));
    let catalog = crate::tools::core::bundled_catalog(&rows).unwrap();
    assert_eq!(catalog["total"], 4);
    assert_eq!(catalog["operation_total"], 8);
    assert!(
        catalog["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| { row["kind"] == "built_in" && row["tags"] == json!(["core"]) })
    );
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM plugin_installations")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(count, 0);
    let after = db
        .set_core_plugin_enabled("core.web", 1, false)
        .await
        .unwrap();
    assert_eq!(after.enabled, 0);
    assert_eq!(after.revision, 2);
    let current = db.core_plugin_settings().await.unwrap();
    assert_eq!(
        current
            .iter()
            .find(|row| row.plugin_id == "core.web")
            .unwrap()
            .enabled,
        0
    );
    runtime.shutdown().await;
    // Connect a new Database handle against the same durable file. Migrations
    // must not reset previously disabled Core groups at server restart.
    let reopened = Database::connect(&config.database).await.unwrap();
    let persisted = reopened.core_plugin_settings().await.unwrap();
    let web = persisted
        .iter()
        .find(|row| row.plugin_id == "core.web")
        .unwrap();
    assert_eq!((web.enabled, web.revision), (0, 2));
}

#[tokio::test]
async fn disabled_groups_are_absent_at_admission_and_old_freezes_do_not_expand() {
    let (db, _, runtime) = harness().await;
    db.set_core_plugin_enabled("core.web", 1, false)
        .await
        .unwrap();
    let job = runtime
        .engine
        .submit(request("Inspect JSON"), &JobRequester::Admin, None)
        .await
        .unwrap()
        .job;
    let choices = job.plugin_choices.as_array().unwrap();
    assert_eq!(choices.len(), 6);
    assert!(!choices.iter().any(|choice| {
        choice["plugin_id"] == "core"
            && choice["operation"]
                .as_str()
                .is_some_and(|op| op.starts_with("web."))
    }));
    let mut needs_web = request("Fetch URL");
    needs_web.required_sources = vec!["web".into()];
    assert_eq!(
        runtime
            .engine
            .submit(needs_web, &JobRequester::Admin, None)
            .await
            .unwrap_err(),
        JobError::CapabilityUnavailable,
    );
    db.set_core_plugin_enabled("core.web", 2, true)
        .await
        .unwrap();
    assert_eq!(
        runtime.engine.get(&job.id).await.unwrap().plugin_choices,
        job.plugin_choices
    );
    runtime.shutdown().await;
}

#[tokio::test]
async fn disabling_blocks_unstarted_step_but_not_a_started_attempt() {
    let (db, _, runtime) = harness().await;
    let job = runtime
        .engine
        .submit(request("Fetch page"), &JobRequester::Admin, None)
        .await
        .unwrap()
        .job;
    let raw = job
        .plugin_choices
        .as_array()
        .unwrap()
        .iter()
        .find(|choice| choice["operation"] == "web.fetch")
        .unwrap();
    let choice = decode_choice(raw).unwrap();
    let args = json!({"url":"https://example.org"});
    db.test_plan_one_step(&job.id, "fetch", &choice, &args)
        .await
        .unwrap();
    db.reserve_plugin_attempt(&job.id, "fetch", "attempt-1")
        .await
        .unwrap();
    db.set_core_plugin_enabled("core.web", 1, false)
        .await
        .unwrap();
    assert_eq!(
        db.request_plugin_start(&job.id, "fetch", "attempt-1", &choice)
            .await,
        Err(JobError::CorePluginDisabled),
    );
    db.finish_plugin_failure(
        &job.id,
        "fetch",
        "attempt-1",
        "core_plugin_disabled",
        false,
        false,
    )
    .await
    .unwrap();
    let (status, _, code) = db.job_step_outcome(&job.id, "fetch").await.unwrap();
    assert_eq!(status, "failed");
    assert_eq!(code.as_deref(), Some("core_plugin_disabled"));

    let other = runtime
        .engine
        .submit(request("JSON query"), &JobRequester::Admin, None)
        .await
        .unwrap()
        .job;
    let data = decode_choice(
        other
            .plugin_choices
            .as_array()
            .unwrap()
            .iter()
            .find(|choice| choice["operation"] == "data.json_query")
            .unwrap(),
    )
    .unwrap();
    let query = json!({"json_text":"{\"ok\":true}","pointer":"/ok"});
    db.test_plan_one_step(&other.id, "data", &data, &query)
        .await
        .unwrap();
    db.reserve_plugin_attempt(&other.id, "data", "attempt-2")
        .await
        .unwrap();
    db.request_plugin_start(&other.id, "data", "attempt-2", &data)
        .await
        .unwrap();
    db.mark_plugin_started(&other.id, "data", "attempt-2")
        .await
        .unwrap();
    // A successfully authorized attempt can complete after its own group
    // is disabled, because authority was committed at start_requested.
    db.set_core_plugin_enabled("core.data", 1, false)
        .await
        .unwrap();
    db.finish_plugin_success(
        &other.id,
        "data",
        "attempt-2",
        &json!({"value":true}),
        &[],
        (1024, 4096, false),
    )
    .await
    .unwrap();
    let (status, _, _) = db.job_step_outcome(&other.id, "data").await.unwrap();
    assert_eq!(status, "succeeded");
    let row = db.job_row(&other.id).await.unwrap();
    assert_eq!(
        super::scheduler::eligible(
            &runtime.engine,
            &row,
            crate::database::unix_seconds().unwrap()
        )
        .await,
        Ok(())
    );
    runtime.shutdown().await;
}

#[tokio::test]
async fn builtin_http_uses_auth_revision_and_static_core_tag() {
    use crate::{
        app::{AppState, router_with_state},
        providers::ProviderSet,
    };
    let (db, config, runtime) = harness().await;
    let mut state = AppState::from_provider_set_with_database(
        (*config).clone(),
        Arc::new(ProviderSet::unavailable()),
        Some((*db).clone()),
    );
    state.background = Some(runtime.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!(
        "http://{}/api/admin/plugins/builtin",
        listener.local_addr().unwrap()
    );
    let server = tokio::spawn(async move {
        axum::serve(listener, router_with_state(state))
            .await
            .unwrap();
    });
    let client = reqwest::Client::new();
    assert_eq!(client.get(&base).send().await.unwrap().status(), 401);
    let catalog: Value = client
        .get(&base)
        .bearer_auth("background-test-token")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(catalog["operation_total"], 8);
    assert_eq!(catalog["items"][0]["tags"], json!(["core"]));
    let patch = || {
        client
            .patch(format!("{base}/core.web"))
            .bearer_auth("background-test-token")
            .header(reqwest::header::IF_MATCH, "\"1\"")
            .json(&json!({"enabled":false}))
    };
    let response = patch().send().await.unwrap();
    assert_eq!(response.status(), 200);
    let updated: Value = response.json().await.unwrap();
    assert_eq!(updated["status"], "disabled");
    assert_eq!(updated["revision"], 2);
    let stale = patch().send().await.unwrap();
    assert_eq!(stale.status(), 409);
    let repeated = client
        .patch(format!("{base}/core.web"))
        .bearer_auth("background-test-token")
        .header(reqwest::header::IF_MATCH, "\"2\"")
        .json(&json!({"enabled":false}))
        .send()
        .await
        .unwrap();
    assert_eq!(repeated.status(), 200);
    assert_eq!(repeated.json::<Value>().await.unwrap()["revision"], 3);
    let invalid = client
        .patch(format!("{base}/core.unknown"))
        .bearer_auth("background-test-token")
        .header(reqwest::header::IF_MATCH, "\"1\"")
        .json(&json!({"enabled":false}))
        .send()
        .await
        .unwrap();
    assert_eq!(invalid.status(), 404);
    let missing_revision = client
        .patch(format!("{base}/core.web"))
        .bearer_auth("background-test-token")
        .json(&json!({"enabled":true}))
        .send()
        .await
        .unwrap();
    assert_eq!(missing_revision.status(), 400);
    let invalid_revision = client
        .patch(format!("{base}/core.web"))
        .bearer_auth("background-test-token")
        .header(reqwest::header::IF_MATCH, "3")
        .json(&json!({"enabled":true}))
        .send()
        .await
        .unwrap();
    assert_eq!(invalid_revision.status(), 400);
    let malformed_body = client
        .patch(format!("{base}/core.web"))
        .bearer_auth("background-test-token")
        .header(reqwest::header::IF_MATCH, "\"3\"")
        .json(&json!({"enabled":"not-a-bool"}))
        .send()
        .await
        .unwrap();
    assert_eq!(malformed_body.status(), 400);
    // A missing UNRELATED catalog row makes GET unavailable, but a valid
    // committed PATCH must still return 200 with its own revision (F1).
    sqlx::query("DELETE FROM core_plugin_settings WHERE plugin_id='core.artifact'")
        .execute(db.pool())
        .await
        .unwrap();
    let successful_without_get = client
        .patch(format!("{base}/core.web"))
        .bearer_auth("background-test-token")
        .header(reqwest::header::IF_MATCH, "\"3\"")
        .json(&json!({"enabled":true}))
        .send()
        .await
        .unwrap();
    assert_eq!(successful_without_get.status(), 200);
    assert_eq!(
        successful_without_get.json::<Value>().await.unwrap()["revision"],
        4
    );
    // A corrupted/missing known settings row must fail closed on GET and PATCH.
    sqlx::query("DELETE FROM core_plugin_settings WHERE plugin_id='core.web'")
        .execute(db.pool())
        .await
        .unwrap();
    let catalog_unavailable = client
        .get(&base)
        .bearer_auth("background-test-token")
        .send()
        .await
        .unwrap();
    assert_eq!(catalog_unavailable.status(), 503);
    let patch_unavailable = client
        .patch(format!("{base}/core.web"))
        .bearer_auth("background-test-token")
        .header(reqwest::header::IF_MATCH, "\"4\"")
        .json(&json!({"enabled":true}))
        .send()
        .await
        .unwrap();
    assert_eq!(patch_unavailable.status(), 503);
    server.abort();
    runtime.shutdown().await;
}

#[tokio::test]
async fn concurrent_core_updates_have_one_cas_winner() {
    let (db, _, runtime) = harness().await;
    let (first, second) = tokio::join!(
        db.set_core_plugin_enabled("core.web", 1, false),
        db.set_core_plugin_enabled("core.web", 1, true),
    );
    assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
    assert_eq!(
        db.core_plugin_settings()
            .await
            .unwrap()
            .iter()
            .find(|row| row.plugin_id == "core.web")
            .unwrap()
            .revision,
        2
    );
    runtime.shutdown().await;
}

#[tokio::test]
async fn missing_known_policy_row_fails_closed_without_reseeding() {
    let (db, _, runtime) = harness().await;
    sqlx::query("DELETE FROM core_plugin_settings WHERE plugin_id='core.web'")
        .execute(db.pool())
        .await
        .unwrap();
    let settings = db.core_plugin_settings().await.unwrap();
    assert!(crate::tools::core::bundled_catalog(&settings).is_err());
    assert_eq!(
        runtime
            .engine
            .submit(request("Try missing policy"), &JobRequester::Admin, None,)
            .await
            .unwrap_err(),
        JobError::DatabaseUnavailable,
    );
    runtime.shutdown().await;
}

#[tokio::test]
async fn step_start_and_admin_disable_race_respects_sqlite_commit_order() {
    // Unlike the sequential authorization cases, both futures compete for
    // the same SQLite write transaction. Either winner is valid, but Step
    // state and the returned error MUST reflect its actual linearization.
    for iteration in 0..8 {
        let (db, _, runtime) = harness().await;
        let job = runtime
            .engine
            .submit(request("Concurrent Core start"), &JobRequester::Admin, None)
            .await
            .unwrap()
            .job;
        let raw = job
            .plugin_choices
            .as_array()
            .unwrap()
            .iter()
            .find(|choice| choice["operation"] == "web.fetch")
            .unwrap();
        let choice = decode_choice(raw).unwrap();
        let step_key = format!("race-{iteration}");
        let attempt = format!("attempt-{iteration}");
        db.test_plan_one_step(
            &job.id,
            &step_key,
            &choice,
            &json!({"url":"https://example.org"}),
        )
        .await
        .unwrap();
        db.reserve_plugin_attempt(&job.id, &step_key, &attempt)
            .await
            .unwrap();
        let (start, disable) = tokio::join!(
            db.request_plugin_start(&job.id, &step_key, &attempt, &choice),
            db.set_core_plugin_enabled("core.web", 1, false),
        );
        assert!(disable.is_ok());
        let attempt_state: String =
            sqlx::query_scalar("SELECT attempt_state FROM job_steps WHERE job_id=? AND step_key=?")
                .bind(&job.id)
                .bind(&step_key)
                .fetch_one(db.pool())
                .await
                .unwrap();
        match start {
            Ok(()) => assert_eq!(attempt_state, "start_requested"),
            Err(JobError::CorePluginDisabled) => assert_eq!(attempt_state, "reserved"),
            Err(other) => panic!("Unexpected concurrent start failure: {other:?}"),
        }
        runtime.shutdown().await;
    }
}

#[tokio::test]
async fn replay_after_enable_retains_frozen_core_authority() {
    let (db, _, runtime) = harness().await;
    db.set_core_plugin_enabled("core.web", 1, false)
        .await
        .unwrap();
    let original = runtime
        .engine
        .submit(
            request("Idempotent frozen Core"),
            &JobRequester::Admin,
            Some("core-frozen-key"),
        )
        .await
        .unwrap();
    assert!(!original.replayed);
    assert!(
        !original
            .job
            .plugin_choices
            .as_array()
            .unwrap()
            .iter()
            .any(|choice| choice["operation"] == "web.fetch")
    );
    db.set_core_plugin_enabled("core.web", 2, true)
        .await
        .unwrap();
    let replayed = runtime
        .engine
        .submit(
            request("Idempotent frozen Core"),
            &JobRequester::Admin,
            Some("core-frozen-key"),
        )
        .await
        .unwrap();
    assert!(replayed.replayed);
    assert_eq!(replayed.job.id, original.job.id);
    assert_eq!(replayed.job.plugin_choices, original.job.plugin_choices);
    assert!(
        runtime
            .engine
            .submit(
                request("A fresh job can see Web"),
                &JobRequester::Admin,
                None,
            )
            .await
            .unwrap()
            .job
            .plugin_choices
            .as_array()
            .unwrap()
            .iter()
            .any(|choice| choice["operation"] == "web.fetch")
    );
    runtime.shutdown().await;
}
