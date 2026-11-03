use std::sync::Arc;
use std::time::Duration;

use sqlx::SqlitePool;
use tokio::net::TcpListener;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use voice_agent_server::{
    app::{AppState, router_with_state},
    config::DatabaseHistoryConfig,
    database::{
        Database,
        history::{RetentionCleaner, unix_millis_now},
    },
    providers::ProviderSet,
    tools::builtin::{EXIT_TOOL_NAME, SWITCH_TEMPLATE_TOOL_NAME},
};

use super::harness::*;

#[tokio::test]
async fn capture_is_off_by_default_so_a_turn_archives_nothing() {
    let harness = start(
        Arc::new(ScriptLlm::new(
            "default-llm",
            vec![Step::Answer("hello from the model")],
        )),
        DatabaseHistoryConfig::default(),
    )
    .await;
    let pool = seed(&harness.url).await;
    assert!(
        harness.metrics.is_none(),
        "capture is opt-in, so the default configuration has no archival writer at all — not an \
         idle one, and no queue for a record to reach"
    );
    let mut socket = admit(&harness.base).await;

    let turn = speak_and_observe(&mut socket).await;

    assert_eq!(turn.answer, "hello from the model");
    assert!(
        rows(&pool).await.is_empty(),
        "capture is opt-in, so the default configuration must not enqueue or write anything"
    );
    harness.task.abort();
}

/// Retention is a property of the archive rather than of capture, so it keeps running with capture
/// off — and it keeps running because the archive is still there, not because a writer was left
/// behind to justify it.
#[tokio::test]
async fn capture_off_still_prunes_what_an_earlier_deployment_archived() {
    let url = database_url();
    let app_config = config(url.clone(), DatabaseHistoryConfig::default());
    let database = Database::connect(&app_config.database).await.unwrap();
    let pool = seed(&url).await;
    let now = unix_millis_now();
    let day: i64 = 24 * 60 * 60 * 1_000;
    sqlx::query(
        "INSERT INTO history_messages \
         (session_id, device_id, agent_id, template_id, sequence, turn_id, role, text, created_at) \
         VALUES ('aged', 1, 1, NULL, 1, '1', 'user', 'expired', ?)",
    )
    .bind(now - 40 * day)
    .execute(&pool)
    .await
    .unwrap();
    let shutdown = tokio_util::sync::CancellationToken::new();
    let _cleaner =
        RetentionCleaner::start(&database, 30, Duration::from_millis(50), shutdown.clone());

    await_rows(&pool, 0).await;
    shutdown.cancel();
}

#[tokio::test]
async fn an_enabled_archive_stores_only_the_final_user_text_and_the_delivered_answer() {
    let harness = start(
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

    let turn = speak_and_observe(&mut socket).await;
    assert_eq!(turn.transcript.as_deref(), Some("utterance"));
    assert_eq!(turn.answer, "the delivered answer");

    let archived = await_rows(&pool, 2).await;
    assert_eq!(
        roles_and_texts(&archived),
        vec![
            ("user".to_owned(), "utterance".to_owned()),
            ("assistant".to_owned(), "the delivered answer".to_owned())
        ],
        "exactly the accepted final user text and the Delivered Assistant Response"
    );
    assert!(
        archived.iter().all(|row| !row.text.contains(PROMPT_MARKER)),
        "the deployed system prompt is never archived"
    );
    assert!(
        archived
            .iter()
            .all(|row| row.turn_id.as_deref() == Some("1")),
        "both records belong to the one turn that produced them"
    );
    assert_eq!(
        archived.iter().map(|row| row.sequence).collect::<Vec<_>>(),
        vec![1, 2],
        "a session numbers its own records, so the archive stays ordered without a query"
    );
    assert!(
        archived[0].session_id == archived[1].session_id,
        "one WebSocket connection is one archive session"
    );
    harness.task.abort();
}

#[tokio::test]
async fn a_turn_that_speaks_a_tool_argument_never_archives_it() {
    let harness = start(
        Arc::new(ScriptLlm::new(
            "default-llm",
            vec![Step::Call {
                name: EXIT_TOOL_NAME,
                arguments: serde_json::json!({"say_goodbye": GOODBYE_ARGUMENT}),
            }],
        )),
        DatabaseHistoryConfig {
            enabled: true,
            ..Default::default()
        },
    )
    .await;
    let pool = seed(&harness.url).await;
    let mut socket = admit(&harness.base).await;

    let turn = speak_and_observe(&mut socket).await;
    assert_eq!(
        turn.answer, GOODBYE_ARGUMENT,
        "the tool's own answer really was delivered to the client"
    );
    assert!(
        turn.audio_frames > 0,
        "the direct tool result must include audio"
    );

    let archived = await_rows(&pool, 1).await;
    assert_eq!(
        roles_and_texts(&archived),
        vec![("user".to_owned(), "utterance".to_owned())],
        "a delivered tool result is not a Delivered Assistant Response"
    );
    assert!(
        !roles_and_texts(&archived)
            .iter()
            .any(|(_, text)| text.contains(GOODBYE_ARGUMENT)),
        "a tool argument the model chose must never reach the archive"
    );
    harness.task.abort();
}

#[tokio::test]
async fn an_interrupted_turn_archives_its_user_text_and_no_answer() {
    let (llm, asked) = ScriptLlm::stalling(
        "default-llm",
        vec![Step::Answer("answered after the interrupt")],
        Duration::from_millis(750),
    );
    let harness = start(
        Arc::new(llm),
        DatabaseHistoryConfig {
            enabled: true,
            ..Default::default()
        },
    )
    .await;
    let pool = seed(&harness.url).await;
    let mut socket = admit(&harness.base).await;

    // Start a turn and interrupt it while the model is stalled on its first request: the turn is
    // provably past its user text and short of its answer, and the stall is long enough that the
    // abort is applied long before the answer could have been delivered.
    socket
        .send(Message::Text(
            serde_json::json!({"type": "listen", "state": "start", "mode": "manual"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    socket
        .send(Message::Binary(canonical_opus_packet().into()))
        .await
        .unwrap();
    socket
        .send(Message::Text(
            serde_json::json!({"type": "listen", "state": "stop"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    asked.notified().await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    socket
        .send(Message::Text(
            serde_json::json!({"type": "abort"}).to_string().into(),
        ))
        .await
        .unwrap();
    // The stall expires on its own; the answer it releases is discarded by the interrupted turn, so
    // the test waits for the archive rather than for a client-visible outcome that never comes.

    let archived = await_rows(&pool, 1).await;
    assert_eq!(
        roles_and_texts(&archived),
        vec![("user".to_owned(), "utterance".to_owned())],
        "a partial exchange is accepted: the user text that was accepted stays, the answer that \
         was never delivered does not exist"
    );

    // The same session keeps working, and its next exchange is archived under its own numbers.
    let resumed = speak_and_observe(&mut socket).await;
    assert_eq!(resumed.answer, "answered after the interrupt");
    let resumed = await_rows(&pool, 3).await;
    assert_eq!(
        resumed.iter().map(|row| row.sequence).collect::<Vec<_>>(),
        vec![1, 2, 3],
        "a dropped or absent answer leaves no hole in the session's own numbering"
    );
    harness.task.abort();
}

#[tokio::test]
async fn a_switched_template_is_attributed_to_the_turn_that_ran_under_it() {
    let harness = start(
        Arc::new(ScriptLlm::new(
            "default-llm",
            vec![Step::Call {
                name: SWITCH_TEMPLATE_TOOL_NAME,
                arguments: serde_json::json!({"template": "sales"}),
            }],
        )),
        DatabaseHistoryConfig {
            enabled: true,
            ..Default::default()
        },
    )
    .await;
    let pool = seed(&harness.url).await;
    let (primary, secondary) = seed_switchable_pair(&pool).await;
    let mut socket = admit(&harness.base).await;

    let switching = speak_and_observe(&mut socket).await;
    assert_eq!(switching.answer, "default-llm[continued]");
    let switched = speak_and_observe(&mut socket).await;
    assert_eq!(switched.answer, "default-llm[continued]");

    let archived = await_rows(&pool, 4).await;
    assert_eq!(
        archived
            .iter()
            .map(|row| (row.turn_id.as_deref(), row.template_id))
            .collect::<Vec<_>>(),
        vec![
            (Some("1"), Some(primary)),
            (Some("1"), Some(primary)),
            (Some("2"), Some(secondary)),
            (Some("2"), Some(secondary)),
        ],
        "a switch armed but not yet applied leaves the switching turn attributed to the Template \
         it ran on, and the next turn to the one it ran under"
    );
    assert_eq!(
        archived
            .iter()
            .map(|row| row.text.as_str())
            .collect::<Vec<_>>(),
        vec![
            "utterance",
            "default-llm[continued]",
            "utterance",
            "default-llm[continued]"
        ],
        "the switching turn ran a tool round, and neither the call's arguments nor its result \
         appears among its two archived texts"
    );
    harness.task.abort();
}

#[tokio::test]
async fn a_server_default_session_archives_without_a_template() {
    let harness = start(
        Arc::new(ScriptLlm::new(
            "default-llm",
            vec![Step::Answer("no template here")],
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
    assert!(
        archived.iter().all(|row| row.template_id.is_none()),
        "an Agent with no assignment archives with no Template rather than a fabricated one"
    );
    harness.task.abort();
}

#[tokio::test]
async fn an_unknown_device_cannot_start_a_session_or_archive_anything() {
    let url = database_url();
    let mut app_config = config(
        url.clone(),
        DatabaseHistoryConfig {
            enabled: true,
            ..Default::default()
        },
    );
    let database = Database::connect(&app_config.database).await.unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    app_config.server.bind = address;
    let state = AppState::from_provider_set_with_database(
        app_config,
        Arc::new(ProviderSet::with_all(
            Arc::new(SilentVad),
            Arc::new(FinalAsr),
            Arc::new(ScriptLlm::new(
                "default-llm",
                vec![Step::Answer("no admission identity")],
            )),
            Arc::new(ShortTts),
        )),
        Some(database),
    );
    let task = tokio::spawn(async move {
        axum::serve(listener, router_with_state(state))
            .await
            .unwrap()
    });
    let pool = SqlitePool::connect(&url).await.unwrap();

    let error = connect_async(request(&format!("http://{address}")))
        .await
        .unwrap_err();
    match error {
        tokio_tungstenite::tungstenite::Error::Http(response) => {
            assert_eq!(response.status(), 403);
        }
        other => panic!("expected admission rejection, got {other:?}"),
    }

    assert!(
        rows(&pool).await.is_empty(),
        "an archive row belongs to an admitted Device and Agent, and this session has neither"
    );
    task.abort();
}
