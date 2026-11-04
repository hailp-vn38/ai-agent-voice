use std::time::Duration;

use sqlx::SqlitePool;
use vela_server::{
    config::DatabaseHistoryConfig,
    database::{
        Database,
        history::{RetentionCleaner, unix_millis_now},
    },
};

use super::harness::*;

#[tokio::test]
async fn retention_runs_at_startup_and_again_on_its_schedule() {
    let url = database_url();
    let app_config = config(
        url.clone(),
        DatabaseHistoryConfig {
            retention_days: 30,
            ..Default::default()
        },
    );
    let database = Database::connect(&app_config.database).await.unwrap();
    let pool = seed(&url).await;
    let now = unix_millis_now();
    let day: i64 = 24 * 60 * 60 * 1_000;
    let insert = |sequence: i64, created_at: i64, text: &str| {
        let pool = pool.clone();
        let text = text.to_owned();
        async move {
            sqlx::query(
                "INSERT INTO history_messages \
                 (session_id, device_id, agent_id, template_id, sequence, turn_id, role, text, created_at) \
                 VALUES ('aged', 1, 1, NULL, ?, '1', 'user', ?, ?)",
            )
            .bind(sequence)
            .bind(text)
            .bind(created_at)
            .execute(&pool)
            .await
            .unwrap();
        }
    };
    insert(1, now - 40 * day, "expired before the first run").await;
    insert(2, now, "kept by the first run").await;
    let shutdown = tokio_util::sync::CancellationToken::new();
    let _retention =
        RetentionCleaner::start(&database, 30, Duration::from_millis(50), shutdown.clone());

    await_rows(&pool, 1).await;
    assert_eq!(
        roles_and_texts(&rows(&pool).await),
        vec![("user".to_owned(), "kept by the first run".to_owned())],
        "a record inside the retention window survives the startup run"
    );

    // A second expired record can only disappear if the schedule fired again.
    insert(3, now - 31 * day, "expired after the first run").await;
    let mut remaining = Vec::new();
    for _ in 0..240 {
        remaining = rows(&pool).await;
        if remaining.len() == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(
        roles_and_texts(&remaining),
        vec![("user".to_owned(), "kept by the first run".to_owned())],
        "retention keeps deleting on its schedule, not only once at startup"
    );
    shutdown.cancel();
}

#[tokio::test]
async fn a_contended_retention_run_is_abandoned_and_the_next_one_still_happens() {
    let url = database_url();
    let app_config = config(
        url.clone(),
        DatabaseHistoryConfig {
            retention_days: 30,
            ..Default::default()
        },
    );
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

    // A separate owner holds the write lock for longer than the busy timeout, so the startup run
    // is contended and abandoned rather than waited on or retried.
    let blocking = SqlitePool::connect(&url).await.unwrap();
    let mut connection = blocking.acquire().await.unwrap();
    sqlx::query("BEGIN EXCLUSIVE")
        .execute(&mut *connection)
        .await
        .unwrap();
    let shutdown = tokio_util::sync::CancellationToken::new();
    let _retention =
        RetentionCleaner::start(&database, 30, Duration::from_millis(50), shutdown.clone());
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        rows(&pool).await.len(),
        1,
        "a contended run must not hold the lock the Voice Sessions' own writes need"
    );
    sqlx::query("ROLLBACK")
        .execute(&mut *connection)
        .await
        .unwrap();
    drop(connection);
    drop(blocking);

    let mut remaining = Vec::new();
    for _ in 0..240 {
        remaining = rows(&pool).await;
        if remaining.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(
        remaining.is_empty(),
        "the abandoned run is not retried in a loop; the next scheduled run does the work"
    );
    shutdown.cancel();
}
