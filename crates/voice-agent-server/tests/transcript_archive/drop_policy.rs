use std::sync::Arc;

use sqlx::SqlitePool;
use voice_agent_server::config::DatabaseHistoryConfig;

use super::harness::*;

/// Holds the database's single write lock, so the archival writer cannot finish a record and the
/// bounded hand-off is what a record actually meets.
async fn lock_writes(pool: &SqlitePool) -> sqlx::pool::PoolConnection<sqlx::Sqlite> {
    let mut connection = pool.acquire().await.unwrap();
    sqlx::query("BEGIN EXCLUSIVE")
        .execute(&mut *connection)
        .await
        .unwrap();
    connection
}

#[tokio::test]
async fn a_full_archive_queue_drops_one_record_and_never_the_turn() {
    let harness = start_with_database(
        Arc::new(ScriptLlm::new(
            "default-llm",
            vec![Step::Answer("first answer"), Step::Answer("second answer")],
        )),
        DatabaseHistoryConfig {
            enabled: true,
            queue_capacity: 1,
            ..Default::default()
        },
        30_000,
    )
    .await;
    let pool = seed(&harness.url).await;
    let metrics = harness.metrics.clone().expect("a database owns an archive");
    let mut socket = admit(&harness.base).await;
    let mut write_lock = lock_writes(&pool).await;

    // The writer is stuck on its first record, so the single queue slot is all the room the
    // archive has: the rest of these turns' records have nowhere to go but be dropped.
    let first = speak_and_observe(&mut socket).await;
    let second = speak_and_observe(&mut socket).await;

    assert_eq!(first.answer, "first answer");
    assert_eq!(second.answer, "second answer");
    assert!(
        first.audio_frames > 0 && second.audio_frames > 0,
        "a blocked archive must not cost the client a single audio frame"
    );
    await_drops(&metrics, 1).await;
    assert!(
        metrics.counters().dropped_queue_full >= 1,
        "the record that met a full queue is counted as such, not as a database failure"
    );

    sqlx::query("ROLLBACK")
        .execute(&mut *write_lock)
        .await
        .unwrap();
    await_settled(&metrics).await;
    let counters = metrics.counters();
    let archived = rows(&pool).await;
    assert_eq!(
        archived.len() as u64,
        counters.written,
        "the archive holds exactly what the writer stored"
    );
    assert_eq!(
        counters.enqueued + counters.dropped_queue_full,
        4,
        "four records were produced, every one either reached the hand-off or was refused by it"
    );
    assert_eq!(
        counters.dropped_database, 0,
        "a queue that refused a record is not a database failure"
    );
    assert!(
        archived
            .iter()
            .all(|row| row.text == "utterance" || row.text.ends_with("answer")),
        "a dropped record leaves a gap in the archive; it never leaves a different text in it"
    );
    harness.task.abort();
}

#[tokio::test]
async fn a_database_that_refuses_a_record_drops_it_and_never_the_turn() {
    let harness = start(
        Arc::new(ScriptLlm::new(
            "default-llm",
            vec![Step::Answer("first"), Step::Answer("second")],
        )),
        DatabaseHistoryConfig {
            enabled: true,
            ..Default::default()
        },
    )
    .await;
    let pool = seed(&harness.url).await;
    let metrics = harness.metrics.clone().expect("a database owns an archive");
    let mut socket = admit(&harness.base).await;

    speak_and_observe(&mut socket).await;
    let _ = await_rows(&pool, 2).await;

    // An archive row belongs to its Device and Agent by foreign key, so removing the Device
    // leaves the writer with a record the archive cannot possibly hold.
    sqlx::query("DELETE FROM devices")
        .execute(&pool)
        .await
        .unwrap();
    let answered = speak_and_observe(&mut socket).await;

    assert_eq!(
        answered.answer, "second",
        "a turn the archive cannot record is still answered in full"
    );
    assert!(answered.audio_frames > 0);
    await_drops(&metrics, 2).await;
    assert_eq!(
        metrics.counters().dropped_database,
        2,
        "both records of the unarchivable turn are counted as database failures"
    );
    harness.task.abort();
}
