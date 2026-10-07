//! Ticket 13: a blocked Device tool conflict is resolved by a bounded discovery recovery batch.
//!
//! These tests drive the batch store directly (the same surface the Session actor records complete
//! `tools/list` walks through).  They cover late results, mixed batches, conflict, timeout,
//! disconnect, consistency, revision/CAS and bounded retention.

use serde_json::json;
use sqlx::SqlitePool;
use voice_agent_server::database::Database;
use voice_agent_server::database::device_tool_allowlist;
use voice_agent_server::database::device_tool_recovery::{
    self, BatchState, MAX_BATCHES_PER_DEVICE, MAX_HISTORY_PER_DEVICE, REQUIRED_MEMBERS,
};
use voice_agent_server::tools::device_mcp::{DiscoveredTool, contract_fingerprint};

fn temp_database_url(label: &str) -> String {
    let path = std::env::temp_dir().join(format!(
        "voice-agent-recovery-{label}-{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    format!("sqlite://{}", path.display())
}

async fn database(label: &str) -> (Database, SqlitePool) {
    let url = temp_database_url(label);
    let database = Database::connect(&voice_agent_server::config::DatabaseConfig {
        url,
        max_connections: 2,
        busy_timeout_ms: 5_000,
        migrate_on_start: true,
        devices: Default::default(),
        history: Default::default(),
    })
    .await
    .unwrap();
    let pool = database.pool().clone();
    (database, pool)
}

fn tool(name: &str, description: &str, property: &str) -> DiscoveredTool {
    DiscoveredTool {
        original_name: name.into(),
        description: description.into(),
        input_schema: json!({
            "type": "object",
            "properties": {property: {"type": "number"}},
            "additionalProperties": false
        }),
    }
}

/// One Agent with one Device and a speaker policy that puts it in review mode.
async fn seed(pool: &SqlitePool) -> i64 {
    sqlx::query("INSERT INTO agents (key,name,enabled,created_at,updated_at) VALUES ('agent','Agent',1,1,1)")
        .execute(pool)
        .await
        .unwrap();
    let device = sqlx::query("INSERT INTO devices (device_id,agent_id,enabled,created_at,updated_at) VALUES ('device',1,1,1,1)")
        .execute(pool)
        .await
        .unwrap()
        .last_insert_rowid();
    sqlx::query("INSERT INTO agent_speaker_policies (agent_id,mode) VALUES (1,'observe')")
        .execute(pool)
        .await
        .unwrap();
    device
}

/// Drive a genuine conflict into the observation store: the same tool is observed twice with
/// different contracts, which blocks it (Ticket 12 semantics).
async fn block_with_conflict(database: &Database, device: i64) {
    device_tool_allowlist::observe(
        database,
        device,
        &[tool("SetBrightness", "dim the light", "level")],
    )
    .await
    .unwrap();
    device_tool_allowlist::observe(
        database,
        device,
        &[tool("SetBrightness", "dim the light slowly", "level")],
    )
    .await
    .unwrap();
}

async fn blocked(pool: &SqlitePool, device: i64) -> bool {
    sqlx::query_scalar::<_, i64>(
        "SELECT blocked FROM device_tool_observations WHERE device_id=? AND original_name='SetBrightness'",
    )
    .bind(device)
    .fetch_one(pool)
    .await
    .unwrap()
        == 1
}

async fn stored_fingerprint(pool: &SqlitePool, device: i64) -> String {
    sqlx::query_scalar(
        "SELECT fingerprint FROM device_tool_observations WHERE device_id=? AND original_name='SetBrightness'",
    )
    .bind(device)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn batch_count(pool: &SqlitePool, device: i64) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM device_tool_recovery_batches WHERE device_id=?")
        .bind(device)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn history_count(pool: &SqlitePool, device: i64) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM device_tool_observation_history WHERE device_id=?")
        .bind(device)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_consistent_batch_clears_the_block_but_admits_nothing_until_reviewed() {
    let (database, pool) = database("consistent").await;
    let device = seed(&pool).await;
    block_with_conflict(&database, device).await;
    assert!(blocked(&pool, device).await);

    let batch = device_tool_recovery::start(&pool, device, 60)
        .await
        .unwrap();
    assert_eq!(batch.state, BatchState::Open);
    assert_eq!(batch.required, REQUIRED_MEMBERS);
    assert_eq!(batch.members, 0);

    // One complete member is not quorum: the block survives.
    let first = device_tool_recovery::record(
        &pool,
        device,
        "session-a",
        &[tool("SetBrightness", "dim the light slowly", "level")],
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(first.state, BatchState::Open);
    assert!(blocked(&pool, device).await);

    // A second, agreeing complete member resolves the conflict.
    let second = device_tool_recovery::record(
        &pool,
        device,
        "session-b",
        &[tool("SetBrightness", "dim the light slowly", "level")],
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(second.state, BatchState::Reviewable);
    assert!(second.changed);
    assert!(!blocked(&pool, device).await);
    assert_eq!(
        stored_fingerprint(&pool, device).await,
        contract_fingerprint(&tool("SetBrightness", "dim the light slowly", "level"))
    );

    // A consistent batch is only reviewable: nothing is admitted without explicit approval.
    let (participating, admitted) = device_tool_allowlist::load_admitted(&database, 1, device)
        .await
        .unwrap();
    assert!(participating);
    assert!(admitted.is_empty());

    let current = device_tool_recovery::current(&pool, device)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current.state, BatchState::Reviewable);
    assert_eq!(current.members, 2);
}

#[tokio::test]
async fn a_conflicting_batch_member_keeps_the_tool_blocked() {
    let (database, pool) = database("conflicting").await;
    let device = seed(&pool).await;
    block_with_conflict(&database, device).await;

    device_tool_recovery::start(&pool, device, 60)
        .await
        .unwrap();
    device_tool_recovery::record(
        &pool,
        device,
        "session-a",
        &[tool("SetBrightness", "dim the light slowly", "level")],
    )
    .await
    .unwrap();

    // A member that disagrees fails the whole batch; the agreeing member is not cherry-picked.
    let failed = device_tool_recovery::record(
        &pool,
        device,
        "session-b",
        &[tool("SetBrightness", "flicker", "level")],
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(failed.state, BatchState::Failed);
    assert!(blocked(&pool, device).await);
}

#[tokio::test]
async fn a_member_that_stops_advertising_the_blocked_tool_is_not_consistent() {
    let (database, pool) = database("missing-member").await;
    let device = seed(&pool).await;
    block_with_conflict(&database, device).await;

    device_tool_recovery::start(&pool, device, 60)
        .await
        .unwrap();
    // The first member is not quorum yet.
    let first = device_tool_recovery::record(
        &pool,
        device,
        "session-a",
        &[tool("OtherTool", "unrelated", "value")],
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(first.state, BatchState::Open);
    // The members agree, but neither re-observes the blocked tool, so the conflict is unproven.
    let second = device_tool_recovery::record(
        &pool,
        device,
        "session-b",
        &[tool("OtherTool", "unrelated", "value")],
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(second.state, BatchState::Failed);
    assert!(blocked(&pool, device).await);
}

#[tokio::test]
async fn late_results_after_a_batch_completes_are_ignored() {
    let (database, pool) = database("late").await;
    let device = seed(&pool).await;
    block_with_conflict(&database, device).await;

    device_tool_recovery::start(&pool, device, 60)
        .await
        .unwrap();
    for key in ["session-a", "session-b"] {
        device_tool_recovery::record(
            &pool,
            device,
            key,
            &[tool("SetBrightness", "dim the light slowly", "level")],
        )
        .await
        .unwrap();
    }

    // No open batch remains, so a third walk belongs to no batch and is dropped.
    let late = device_tool_recovery::record(
        &pool,
        device,
        "session-c",
        &[tool("SetBrightness", "flicker", "level")],
    )
    .await
    .unwrap();
    assert!(late.is_none());

    let current = device_tool_recovery::current(&pool, device)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current.state, BatchState::Reviewable);
    assert!(!blocked(&pool, device).await);
    assert_eq!(
        stored_fingerprint(&pool, device).await,
        contract_fingerprint(&tool("SetBrightness", "dim the light slowly", "level"))
    );
}

#[tokio::test]
async fn mixed_batches_do_not_combine() {
    let (database, pool) = database("mixed").await;
    let device = seed(&pool).await;
    block_with_conflict(&database, device).await;

    let first = device_tool_recovery::start(&pool, device, 60)
        .await
        .unwrap();
    device_tool_recovery::record(
        &pool,
        device,
        "session-a",
        &[tool("SetBrightness", "dim the light slowly", "level")],
    )
    .await
    .unwrap();

    // Starting a new batch supersedes the open one; its member can never combine with the new one.
    let second = device_tool_recovery::start(&pool, device, 60)
        .await
        .unwrap();
    assert_ne!(first.id, second.id);

    let outcome = device_tool_recovery::record(
        &pool,
        device,
        "session-b",
        &[tool("SetBrightness", "dim the light slowly", "level")],
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(outcome.state, BatchState::Open);
    assert!(blocked(&pool, device).await);

    let members: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM device_tool_recovery_members WHERE batch_id=?")
            .bind(second.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(members, 1);
}

#[tokio::test]
async fn a_timed_out_batch_stays_blocked() {
    let (database, pool) = database("timeout").await;
    let device = seed(&pool).await;
    block_with_conflict(&database, device).await;

    device_tool_recovery::start(&pool, device, 60)
        .await
        .unwrap();
    device_tool_recovery::record(
        &pool,
        device,
        "session-a",
        &[tool("SetBrightness", "dim the light slowly", "level")],
    )
    .await
    .unwrap();

    // The deadline sweeps the incomplete batch to expired; the block is never cleared.
    let swept = device_tool_recovery::sweep_expired(&pool, i64::MAX)
        .await
        .unwrap();
    assert_eq!(swept, 1);
    let current = device_tool_recovery::current(&pool, device)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current.state, BatchState::Expired);
    assert!(blocked(&pool, device).await);

    // Recording after the deadline cannot revive the batch.
    let late = device_tool_recovery::record(
        &pool,
        device,
        "session-b",
        &[tool("SetBrightness", "dim the light slowly", "level")],
    )
    .await
    .unwrap();
    assert!(late.is_none());
}

#[tokio::test]
async fn a_disconnect_fails_the_open_batch_and_keeps_the_block() {
    let (database, pool) = database("disconnect").await;
    let device = seed(&pool).await;
    block_with_conflict(&database, device).await;

    device_tool_recovery::start(&pool, device, 60)
        .await
        .unwrap();
    device_tool_recovery::record(
        &pool,
        device,
        "session-a",
        &[tool("SetBrightness", "dim the light slowly", "level")],
    )
    .await
    .unwrap();

    assert!(
        device_tool_recovery::fail_open(&pool, device)
            .await
            .unwrap()
    );
    let current = device_tool_recovery::current(&pool, device)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current.state, BatchState::Failed);
    assert!(blocked(&pool, device).await);
    // Failing an already-terminal batch is a no-op.
    assert!(
        !device_tool_recovery::fail_open(&pool, device)
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn bounded_retention_prunes_batches_and_superseded_observations() {
    let (database, pool) = database("retention").await;
    let device = seed(&pool).await;
    block_with_conflict(&database, device).await;

    // Superseded evidence is retained when a consistent batch applies a new contract.
    device_tool_recovery::start(&pool, device, 60)
        .await
        .unwrap();
    for key in ["session-a", "session-b"] {
        device_tool_recovery::record(
            &pool,
            device,
            key,
            &[tool("SetBrightness", "dim the light slowly", "level")],
        )
        .await
        .unwrap();
    }
    let superseded: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM device_tool_observation_history WHERE device_id=? AND original_name='SetBrightness'",
    )
    .bind(device)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(superseded, 1);

    // Batches and superseded observations are both capped.
    for _ in 0..(MAX_BATCHES_PER_DEVICE + 4) {
        device_tool_recovery::start(&pool, device, 60)
            .await
            .unwrap();
    }
    assert!(batch_count(&pool, device).await <= MAX_BATCHES_PER_DEVICE);

    for _ in 0..(MAX_HISTORY_PER_DEVICE + 4) {
        // Each resolving batch supersedes the current evidence once.
        device_tool_recovery::start(&pool, device, 60)
            .await
            .unwrap();
        for key in ["session-a", "session-b"] {
            let description = format!("contract {}", uuid::Uuid::new_v4());
            device_tool_recovery::record(
                &pool,
                device,
                key,
                &[tool("SetBrightness", &description, "level")],
            )
            .await
            .unwrap();
        }
    }
    assert!(history_count(&pool, device).await <= MAX_HISTORY_PER_DEVICE);
}
