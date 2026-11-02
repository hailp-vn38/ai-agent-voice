use super::*;
use crate::{config::DatabaseConfig, database::wiki_conversations};

async fn open_database() -> (Database, std::path::PathBuf) {
    let path = std::env::temp_dir().join(format!(
        "wiki-conversations-limits-{}.db",
        uuid::Uuid::new_v4()
    ));
    let database = Database::connect(&DatabaseConfig {
        url: format!("sqlite://{}", path.display()),
        max_connections: 4,
        busy_timeout_ms: 5_000,
        migrate_on_start: true,
        devices: Default::default(),
        history: Default::default(),
    })
    .await
    .expect("open test database");
    (database, path)
}

fn remove_database(path: &std::path::Path) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

async fn start_turn(
    database: &Database,
    index: usize,
    conversation_id: Option<&str>,
) -> wiki_conversations::StartedTurn {
    wiki_conversations::start_question(
        database,
        conversation_id,
        &format!("conversation-{index}"),
        &format!("user-{index}"),
        &format!("assistant-{index}"),
        &format!("question {index}"),
        &[],
        index as i64,
    )
    .await
    .expect("start question")
}

async fn finish_turn(database: &Database, turn: &wiki_conversations::StartedTurn) {
    wiki_conversations::finish_answer(
        database,
        &turn.conversation_id,
        &turn.assistant_message_id,
        "answer",
        "done",
        None,
        &[],
        &[],
        None,
        turn.user_seq,
    )
    .await
    .expect("finish answer");
}

#[tokio::test]
async fn creating_the_201st_conversation_evicts_the_oldest_idle_conversation() {
    let (database, path) = open_database().await;
    for index in 0..200 {
        let turn = start_turn(&database, index, None).await;
        if index != 0 {
            finish_turn(&database, &turn).await;
        }
    }

    let overflow = start_turn(&database, 200, None).await;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM wiki_conversations")
        .fetch_one(database.pool())
        .await
        .unwrap();
    let oldest_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM wiki_conversations WHERE id = 'conversation-0')",
    )
    .fetch_one(database.pool())
    .await
    .unwrap();
    let next_oldest_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM wiki_conversations WHERE id = 'conversation-1')",
    )
    .fetch_one(database.pool())
    .await
    .unwrap();

    assert_eq!(count, 200);
    assert!(oldest_exists, "a running conversation must not be evicted");
    assert!(
        !next_oldest_exists,
        "the oldest idle conversation is evicted"
    );
    assert_eq!(overflow.conversation_id, "conversation-200");

    drop(database);
    remove_database(&path);
}

#[tokio::test]
async fn a_conversation_stops_at_one_hundred_messages_without_writing_a_new_turn() {
    let (database, path) = open_database().await;
    let mut conversation_id = None;
    for index in 0..50 {
        let turn = start_turn(&database, index, conversation_id.as_deref()).await;
        conversation_id = Some(turn.conversation_id.clone());
        finish_turn(&database, &turn).await;
    }

    let rejected = wiki_conversations::start_question(
        &database,
        conversation_id.as_deref(),
        "overflow-conversation",
        "overflow-user",
        "overflow-assistant",
        "one more question",
        &[],
        1000,
    )
    .await;
    assert!(matches!(
        rejected,
        Err(wiki_conversations::StartTurnError::Full)
    ));

    let message_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM wiki_conversation_messages WHERE conversation_id = ?",
    )
    .bind(conversation_id.as_deref().unwrap())
    .fetch_one(database.pool())
    .await
    .unwrap();
    assert_eq!(message_count, 100);

    drop(database);
    remove_database(&path);
}

#[tokio::test]
async fn saved_answers_tools_and_graphs_respect_their_byte_limits() {
    let (database, path) = open_database().await;
    let turn = start_turn(&database, 0, None).await;
    let answer = format!(
        "{}漢",
        "a".repeat(wiki_conversations::MAX_STORED_ANSWER_BYTES - 1)
    );
    let tools = vec![
        wiki_conversations::ToolStep {
            call_id: "first".to_owned(),
            name: "wiki_get".to_owned(),
            state: "completed".to_owned(),
            query: Some("a".repeat(40_000)),
            pages: vec!["wiki/a.md".to_owned()],
        },
        wiki_conversations::ToolStep {
            call_id: "second".to_owned(),
            name: "wiki_get".to_owned(),
            state: "completed".to_owned(),
            query: Some("b".repeat(40_000)),
            pages: vec!["wiki/b.md".to_owned()],
        },
    ];
    let graph = serde_json::json!({"description": "g".repeat(300 * 1024)});
    wiki_conversations::finish_answer(
        &database,
        &turn.conversation_id,
        &turn.assistant_message_id,
        &answer,
        "done",
        None,
        &tools,
        &["wiki/cited.md".to_owned()],
        Some(&graph),
        10,
    )
    .await
    .expect("finish bounded answer");

    let saved = wiki_conversations::get_conversation(&database, &turn.conversation_id)
        .await
        .unwrap()
        .unwrap();
    let assistant = &saved.messages[1];
    assert_eq!(assistant.status, "done");
    assert!(assistant.content_truncated);
    assert_eq!(
        assistant.content.len(),
        wiki_conversations::MAX_STORED_ANSWER_BYTES - 1
    );
    assert!(assistant.content.is_char_boundary(assistant.content.len()));
    assert_eq!(assistant.citations, vec!["wiki/cited.md".to_owned()]);
    assert_eq!(assistant.tools.len(), 1, "steps are dropped from the end");
    assert_eq!(assistant.tools[0].call_id, "first");
    assert!(
        assistant.graph.is_none(),
        "oversized graph snapshots are omitted"
    );

    drop(database);
    remove_database(&path);
}
