use std::sync::Arc;

use reqwest::StatusCode;
use vela_server::{config::DatabaseHistoryConfig, database::history::unix_millis_now};

use super::harness::*;

fn admin(base: &str, path: &str) -> String {
    format!("{base}/api/admin{path}")
}

async fn purge_body(harness: &Harness, body: serde_json::Value) -> reqwest::Response {
    reqwest::Client::new()
        .post(admin(&harness.base, "/history/purge"))
        .bearer_auth(&harness.token)
        .json(&body)
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn the_archive_is_readable_and_prunable_through_the_authenticated_admin_api() {
    let harness = start_prepared(
        Arc::new(ScriptLlm::new(
            "default-llm",
            vec![Step::Answer("the delivered answer")],
        )),
        DatabaseHistoryConfig {
            enabled: true,
            ..Default::default()
        },
    )
    .await;
    let pool = seed(&harness.url).await;
    let mut socket = admit(&harness.base).await;
    speak_and_observe(&mut socket).await;
    let archived = await_rows(&pool, 2).await;
    let session_id = archived[0].session_id.clone();
    let client = reqwest::Client::new();

    let anonymous = client
        .get(admin(&harness.base, "/history"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        anonymous.status(),
        StatusCode::UNAUTHORIZED,
        "the archive is readable only through the separately authenticated Admin API"
    );
    let anonymous_purge = client
        .post(admin(&harness.base, "/history/purge"))
        .json(&serde_json::json!({"all": "PURGE_ALL_HISTORY"}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        anonymous_purge.status(),
        StatusCode::UNAUTHORIZED,
        "the one destructive operation is behind the same credential as the read"
    );

    let listed: serde_json::Value = client
        .get(admin(&harness.base, "/history"))
        .bearer_auth(&harness.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(listed["items"].as_array().unwrap().len(), 2);
    assert_eq!(listed["max_page_size"], 200);

    for query in [
        ("role=system", StatusCode::BAD_REQUEST),
        ("device_id=0", StatusCode::BAD_REQUEST),
        ("device_id=1; DROP TABLE agents", StatusCode::BAD_REQUEST),
        ("sort=text", StatusCode::BAD_REQUEST),
        ("page_size=1000", StatusCode::BAD_REQUEST),
    ] {
        let response = client
            .get(admin(&harness.base, &format!("/history?{}", query.0)))
            .bearer_auth(&harness.token)
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            query.1,
            "`{}` must be rejected rather than answered with something else",
            query.0
        );
    }
    let users_only: serde_json::Value = client
        .get(admin(
            &harness.base,
            &format!("/history?session_id={session_id}&role=user"),
        ))
        .bearer_auth(&harness.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        users_only["items"].as_array().unwrap().len(),
        1,
        "typed filters narrow the archive without any raw expression reaching SQL"
    );

    for (body, code) in [
        (serde_json::json!({}), "invalid_purge_scope"),
        (
            serde_json::json!({"session_id": session_id, "device_id": 1}),
            "invalid_purge_scope",
        ),
        (serde_json::json!({"all": "all"}), "confirmation_required"),
        (
            serde_json::json!({"all": "all", "confirm": "yes"}),
            "confirmation_required",
        ),
        (
            serde_json::json!({"all": "everything"}),
            "invalid_purge_scope",
        ),
        (
            serde_json::json!({"session_id": session_id, "confirm": "PURGE_ALL_HISTORY"}),
            "invalid_purge_scope",
        ),
        (serde_json::json!({"device_id": 0}), "invalid_purge_scope"),
        (
            serde_json::json!({
                "all": "all",
                "confirm": "PURGE_ALL_HISTORY",
                "session_id": session_id
            }),
            "invalid_purge_scope",
        ),
        (
            serde_json::json!({"session_id": session_id, "agent_id": 1}),
            "invalid_json",
        ),
    ] {
        let response = purge_body(&harness, body.clone()).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "body {body}");
        assert_eq!(
            response.json::<serde_json::Value>().await.unwrap()["error"]["code"],
            code
        );
    }
    assert_eq!(
        rows(&pool).await.len(),
        2,
        "no rejected request deletes anything"
    );

    let purged = purge_body(&harness, serde_json::json!({"session_id": session_id}))
        .await
        .json::<serde_json::Value>()
        .await
        .unwrap();
    assert_eq!(purged["deleted"], 2);
    assert!(rows(&pool).await.is_empty());
    let audit: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM admin_audit_events WHERE resource_type = 'history' AND action = 'purge'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(audit, 1, "a purge leaves exactly one countable trace");
    harness.task.abort();
}

#[tokio::test]
async fn a_purge_never_reaches_an_open_sessions_dialogue_history() {
    let llm = Arc::new(ScriptLlm::new(
        "default-llm",
        vec![Step::Answer("first answer"), Step::Answer("second answer")],
    ));
    let harness = start_prepared(
        llm.clone(),
        DatabaseHistoryConfig {
            enabled: true,
            ..Default::default()
        },
    )
    .await;
    let pool = seed(&harness.url).await;
    let mut socket = admit(&harness.base).await;
    speak_and_observe(&mut socket).await;
    let archived = await_rows(&pool, 2).await;
    let session_id = archived[0].session_id.clone();

    let purged: serde_json::Value =
        purge_body(&harness, serde_json::json!({"session_id": session_id}))
            .await
            .json()
            .await
            .unwrap();
    assert_eq!(purged["deleted"], 2);
    assert!(rows(&pool).await.is_empty());

    let second = speak_and_observe(&mut socket).await;
    assert_eq!(second.answer, "second answer");
    assert_eq!(
        harness.llm.prompt_sizes(),
        vec![2, 4],
        "the archive is empty and the session's prompt is not: the second turn was composed from \
         the system prompt, the first exchange and its own new user text, so the purge left \
         Dialogue History alone"
    );
    let after = await_rows(&pool, 2).await;
    assert_eq!(
        roles_and_texts(&after),
        vec![
            ("user".to_owned(), "utterance".to_owned()),
            ("assistant".to_owned(), "second answer".to_owned())
        ],
        "an open session keeps archiving into a fresh sequence after its earlier records were \
         purged"
    );
    assert_eq!(
        after.iter().map(|row| row.sequence).collect::<Vec<_>>(),
        vec![3, 4],
        "the session's own numbering is its own: a purge emptied the archive without renumbering \
         the session that owns it"
    );
    harness.task.abort();
}

#[tokio::test]
async fn the_archive_stays_readable_and_prunable_while_capture_is_off() {
    let harness = start_prepared(
        Arc::new(ScriptLlm::new("default-llm", Vec::new())),
        DatabaseHistoryConfig::default(),
    )
    .await;
    let pool = seed(&harness.url).await;
    sqlx::query(
        "INSERT INTO history_messages \
         (session_id, device_id, agent_id, template_id, sequence, turn_id, role, text, created_at) \
         VALUES ('archived-by-an-earlier-deployment', 1, 1, NULL, 1, '1', 'user', 'older', ?)",
    )
    .bind(unix_millis_now())
    .execute(&pool)
    .await
    .unwrap();
    let client = reqwest::Client::new();

    let mut socket = admit(&harness.base).await;
    speak_and_observe(&mut socket).await;

    let listed: serde_json::Value = client
        .get(admin(&harness.base, "/history?role=user"))
        .bearer_auth(&harness.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        listed["items"].as_array().unwrap().len(),
        1,
        "capture being off stops new records; it never hides the archive an operator already has"
    );
    assert_eq!(listed["items"][0]["text"], "older");
    let purged: serde_json::Value = purge_body(
        &harness,
        serde_json::json!({"session_id": "archived-by-an-earlier-deployment"}),
    )
    .await
    .json()
    .await
    .unwrap();
    assert_eq!(purged["deleted"], 1);
    harness.task.abort();
}
