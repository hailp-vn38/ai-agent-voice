//! Ticket 12: Device tool contracts are observed, reviewed and enforced.
//!
//! Everything here exercises the real database side of the boundary: a complete validated discovery
//! records evidence, an Agent's explicit approval admits one fingerprint, and a changed contract
//! blocks the tool instead of silently widening the approval.

use serde_json::json;
use sqlx::SqlitePool;
use voice_agent_server::{
    database::{Database, device_tool_allowlist},
    tools::device_mcp::{DiscoveredTool, contract_fingerprint, reviewed_tools},
};

fn temp_database_url(label: &str) -> String {
    let path = std::env::temp_dir().join(format!(
        "voice-agent-device-tools-{label}-{}-{}.db",
        std::process::id(),
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

async fn review(
    database: &Database,
    device: i64,
    name: &str,
    allowed: bool,
    expected_revision: i64,
) {
    let fingerprint: String = sqlx::query_scalar(
        "SELECT fingerprint FROM device_tool_observations WHERE device_id=? AND original_name=?",
    )
    .bind(device)
    .bind(name)
    .fetch_one(database.pool())
    .await
    .unwrap();
    sqlx::query("INSERT INTO agent_device_tool_allowlist (agent_id,device_id,original_name,fingerprint,allowed,sensitive,revision) VALUES (1,?,?,?,?,?,?)")
        .bind(device)
        .bind(name)
        .bind(fingerprint)
        .bind(i64::from(allowed))
        .bind(0_i64)
        .bind(expected_revision)
        .execute(database.pool())
        .await
        .unwrap();
}

#[tokio::test]
async fn a_complete_discovery_is_recorded_but_admits_nothing_until_reviewed() {
    let (database, pool) = database("observation-only").await;
    let device = seed(&pool).await;
    let tools = vec![
        tool("SetBrightness", "set brightness", "level"),
        tool("ReadSensor", "read sensor", "unit"),
    ];
    let (changed, conflicted) = device_tool_allowlist::observe(&database, device, &tools)
        .await
        .unwrap();
    assert!(changed && !conflicted);

    let (participating, contracts) = device_tool_allowlist::load_admitted(&database, 1, device)
        .await
        .unwrap();
    assert!(participating);
    assert!(
        contracts.is_empty(),
        "observation alone must never admit a tool"
    );

    // The evidence is stored, fingerprint included.
    let stored: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM device_tool_observations WHERE device_id=? AND fingerprint=?",
    )
    .bind(device)
    .bind(contract_fingerprint(&tools[0]))
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(stored, 1);
}

#[tokio::test]
async fn approval_admits_exactly_the_approved_fingerprint() {
    let (database, pool) = database("approval").await;
    let device = seed(&pool).await;
    let tools = vec![tool("SetBrightness", "set brightness", "level")];
    device_tool_allowlist::observe(&database, device, &tools)
        .await
        .unwrap();
    review(&database, device, "SetBrightness", true, 1).await;

    let (_, contracts) = device_tool_allowlist::load_admitted(&database, 1, device)
        .await
        .unwrap();
    assert_eq!(
        contracts.get("SetBrightness"),
        Some(&contract_fingerprint(&tools[0]))
    );

    // The admission boundary publishes only the contracted fingerprint.
    let (visible, drift) = reviewed_tools(tools.clone(), &contracts);
    assert!(!drift);
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].original_name, "SetBrightness");
}

#[tokio::test]
async fn an_unreviewed_tool_is_denied_by_default() {
    let (database, pool) = database("unreviewed").await;
    let device = seed(&pool).await;
    let tools = vec![
        tool("SetBrightness", "set brightness", "level"),
        tool("UnlockDoor", "unlock the door", "id"),
    ];
    device_tool_allowlist::observe(&database, device, &tools)
        .await
        .unwrap();
    review(&database, device, "SetBrightness", true, 1).await;

    let (_, contracts) = device_tool_allowlist::load_admitted(&database, 1, device)
        .await
        .unwrap();
    let (visible, _) = reviewed_tools(tools, &contracts);
    let names: Vec<_> = visible
        .iter()
        .map(|tool| tool.original_name.as_str())
        .collect();
    assert_eq!(names, vec!["SetBrightness"]);
}

#[tokio::test]
async fn a_changed_contract_revokes_the_approval_and_blocks_the_tool() {
    let (database, pool) = database("drift").await;
    let device = seed(&pool).await;
    device_tool_allowlist::observe(
        &database,
        device,
        &[tool("SetBrightness", "set brightness", "level")],
    )
    .await
    .unwrap();
    review(&database, device, "SetBrightness", true, 1).await;
    let (_, admitted) = device_tool_allowlist::load_admitted(&database, 1, device)
        .await
        .unwrap();
    assert_eq!(admitted.len(), 1);
    let before: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM device_tool_observations WHERE device_id=? AND blocked=0",
    )
    .bind(device)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(before, 1);

    // The same Device advertises the same name with a different schema.
    let (_, conflicted) = device_tool_allowlist::observe(
        &database,
        device,
        &[tool("SetBrightness", "set brightness", "percent")],
    )
    .await
    .unwrap();
    assert!(
        conflicted,
        "a changed contract must be reported as a conflict"
    );

    let blocked: i64 = sqlx::query_scalar(
        "SELECT blocked FROM device_tool_observations WHERE device_id=? AND original_name='SetBrightness'",
    )
    .bind(device)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(blocked, 1, "a conflicting observation must block the tool");

    let (_, contracts) = device_tool_allowlist::load_admitted(&database, 1, device)
        .await
        .unwrap();
    assert!(
        contracts.is_empty(),
        "a blocked observation must not be admitted"
    );

    // And a session that had already admitted the old contract denies the new one, because the
    // discovered fingerprint no longer matches the admitted one.
    let (visible, drift) = reviewed_tools(
        vec![tool("SetBrightness", "set brightness", "percent")],
        &admitted,
    );
    assert!(visible.is_empty() && drift);
}

#[tokio::test]
async fn a_later_agreeing_discovery_never_clears_a_conflict() {
    let (database, pool) = database("conflict-sticky").await;
    let device = seed(&pool).await;
    device_tool_allowlist::observe(
        &database,
        device,
        &[tool("SetBrightness", "set brightness", "level")],
    )
    .await
    .unwrap();
    device_tool_allowlist::observe(
        &database,
        device,
        &[tool("SetBrightness", "set brightness", "percent")],
    )
    .await
    .unwrap();
    // Back to the original contract: agreement does not resolve the earlier disagreement.
    let (_, conflicted) = device_tool_allowlist::observe(
        &database,
        device,
        &[tool("SetBrightness", "set brightness", "level")],
    )
    .await
    .unwrap();
    assert!(!conflicted);
    let blocked: i64 = sqlx::query_scalar(
        "SELECT blocked FROM device_tool_observations WHERE device_id=? AND original_name='SetBrightness'",
    )
    .bind(device)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(blocked, 1, "only a recovery batch may clear a conflict");
}

#[tokio::test]
async fn a_non_participating_agent_observes_but_keeps_legacy_behavior() {
    let (database, pool) = database("non-participating").await;
    let device = seed(&pool).await;
    sqlx::query("UPDATE agent_speaker_policies SET mode='off' WHERE agent_id=1")
        .execute(&pool)
        .await
        .unwrap();
    let tools = vec![tool("SetBrightness", "set brightness", "level")];
    device_tool_allowlist::observe(&database, device, &tools)
        .await
        .unwrap();

    let (participating, contracts) = device_tool_allowlist::load_admitted(&database, 1, device)
        .await
        .unwrap();
    assert!(!participating);
    assert!(contracts.is_empty(), "no review, no contracted set");

    // The evidence is still recorded for later review.
    let observed: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM device_tool_observations WHERE device_id=?")
            .bind(device)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(observed, 1);
}

#[tokio::test]
async fn deleting_and_re_enrolling_a_device_does_not_inherit_an_approval() {
    let (database, pool) = database("re-enrolment").await;
    let device = seed(&pool).await;
    let tools = vec![tool("SetBrightness", "set brightness", "level")];
    device_tool_allowlist::observe(&database, device, &tools)
        .await
        .unwrap();
    review(&database, device, "SetBrightness", true, 1).await;
    let (_, contracts) = device_tool_allowlist::load_admitted(&database, 1, device)
        .await
        .unwrap();
    assert_eq!(contracts.len(), 1);

    // The Device is removed and re-enrolled: a fresh AUTOINCREMENT id, never the old one.
    sqlx::query("DELETE FROM devices WHERE id=?")
        .bind(device)
        .execute(&pool)
        .await
        .unwrap();
    let re_enrolled = sqlx::query("INSERT INTO devices (device_id,agent_id,enabled,created_at,updated_at) VALUES ('device',1,1,1,1)")
        .execute(&pool)
        .await
        .unwrap()
        .last_insert_rowid();
    assert_ne!(re_enrolled, device);

    let (_, contracts) = device_tool_allowlist::load_admitted(&database, 1, re_enrolled)
        .await
        .unwrap();
    assert!(
        contracts.is_empty(),
        "a re-enrolled Device must not inherit an earlier approval"
    );
}

#[tokio::test]
async fn a_device_tool_and_an_external_tool_may_share_a_name_without_sharing_rights() {
    let (database, pool) = database("same-name").await;
    let device = seed(&pool).await;
    let tools = vec![tool("ReadSensor", "read sensor", "unit")];
    device_tool_allowlist::observe(&database, device, &tools)
        .await
        .unwrap();
    review(&database, device, "ReadSensor", true, 1).await;

    // An External MCP server exposes a tool with the same name, and the Agent declined it.
    let server = sqlx::query("INSERT INTO mcp_servers (key,name,url,created_at,updated_at) VALUES ('sensor','Sensor','http://sensor',1,1)")
        .execute(&pool)
        .await
        .unwrap()
        .last_insert_rowid();
    sqlx::query("INSERT INTO external_tool_observations (server_id,original_name,description,input_schema,fingerprint,server_revision,observed_at) VALUES (?,?,?,?,?,1,1)")
        .bind(server)
        .bind("ReadSensor")
        .bind("read sensor")
        .bind("{}")
        .bind("a".repeat(64))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO agent_external_tool_allowlist (agent_id,server_id,original_name,fingerprint,allowed,sensitive,revision) VALUES (1,?,'ReadSensor',?,0,0,1)")
        .bind(server)
        .bind("a".repeat(64))
        .execute(&pool)
        .await
        .unwrap();

    // The Device approval stands on its own identity; the External decline is untouched.
    let (_, contracts) = device_tool_allowlist::load_admitted(&database, 1, device)
        .await
        .unwrap();
    assert_eq!(
        contracts.get("ReadSensor"),
        Some(&contract_fingerprint(&tools[0]))
    );
    let external_allowed: i64 = sqlx::query_scalar(
        "SELECT allowed FROM agent_external_tool_allowlist WHERE agent_id=1 AND server_id=? AND original_name='ReadSensor'",
    )
    .bind(server)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(external_allowed, 0);
}
