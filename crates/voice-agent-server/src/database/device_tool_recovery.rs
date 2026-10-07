//! Ticket 13: a bounded discovery recovery batch resolves a blocked Device tool conflict.
//!
//! The Session actor already re-runs the normal MCP `tools/list` walk; this store simply groups
//! those complete walks into an admin-started batch scoped to one Device incarnation.  A batch only
//! resolves when enough distinct complete observations agree *and* every blocked tool was
//! re-observed, so a stale, mixed, conflicting or timed-out set can never claim consistency.  A
//! resolved batch is only `reviewable`: it clears the block and adopts the agreed contract, but the
//! Agent's explicit approval is still required before dispatch.
use crate::tools::device_mcp::{DiscoveredTool, contract_fingerprint};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::SqlitePool;
use std::collections::BTreeMap;

/// Distinct complete observations a batch needs before it can resolve.  Two is the minimum that can
/// prove a conflict gone without letting one connection unblock itself (the old latest-wins flaw).
pub const REQUIRED_MEMBERS: i64 = 2;
/// Bounded retention: newest batches kept per Device incarnation.
pub const MAX_BATCHES_PER_DEVICE: i64 = 8;
/// Bounded retention: newest superseded observations kept per Device incarnation.
pub const MAX_HISTORY_PER_DEVICE: i64 = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BatchState {
    /// Collecting complete observations until the quorum agrees or the deadline passes.
    Open,
    /// Every required observation agreed; the block is cleared and the contract awaits review.
    Reviewable,
    /// A member conflicted, or the Device disconnected, or a blocked tool was not re-observed.
    Failed,
    /// The deadline passed with an incomplete batch.
    Expired,
}

impl BatchState {
    fn as_str(self) -> &'static str {
        match self {
            BatchState::Open => "open",
            BatchState::Reviewable => "reviewable",
            BatchState::Failed => "failed",
            BatchState::Expired => "expired",
        }
    }

    fn parse(value: &str) -> BatchState {
        match value {
            "reviewable" => BatchState::Reviewable,
            "failed" => BatchState::Failed,
            "expired" => BatchState::Expired,
            _ => BatchState::Open,
        }
    }
}

/// Current state of a recovery batch, exposed to the admin API/UI.
#[derive(Clone, Debug, Serialize)]
pub struct RecoveryBatch {
    pub id: i64,
    pub state: BatchState,
    pub deadline: i64,
    pub created_at: i64,
    pub completed_at: Option<i64>,
    /// Distinct complete observations recorded so far.
    pub members: i64,
    /// Observations the batch needs to resolve.
    pub required: i64,
}

/// Outcome of recording one complete walk against an open batch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Resolution {
    pub state: BatchState,
    /// A superseding contract was adopted, so existing approvals no longer match.
    pub changed: bool,
}

fn encode(tools: &[DiscoveredTool]) -> String {
    let map = tools
        .iter()
        .map(|tool| {
            (
                tool.original_name.clone(),
                serde_json::json!({
                    "description": tool.description,
                    "input_schema": tool.input_schema,
                    "fingerprint": contract_fingerprint(tool),
                }),
            )
        })
        .collect::<BTreeMap<_, _>>();
    serde_json::to_string(&map).unwrap_or_else(|_| "{}".into())
}

/// `original_name -> fingerprint` view of an encoded member, used to compare observations.
fn fingerprints(encoded: &str) -> BTreeMap<String, String> {
    serde_json::from_str::<BTreeMap<String, Value>>(encoded)
        .unwrap_or_default()
        .into_iter()
        .map(|(name, value)| {
            let fingerprint = value
                .get("fingerprint")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            (name, fingerprint)
        })
        .collect()
}

/// Starts a new recovery batch for a Device incarnation, superseding any batch still open.
pub async fn start(
    database: &SqlitePool,
    device_id: i64,
    deadline_secs: i64,
) -> Result<RecoveryBatch, sqlx::Error> {
    let mut tx = database.begin().await?;
    sqlx::query(
        "UPDATE device_tool_recovery_batches SET state='expired', completed_at=CAST(strftime('%s','now') AS INTEGER) WHERE device_id=? AND state='open'",
    )
    .bind(device_id)
    .execute(&mut *tx)
    .await?;
    let id = sqlx::query(
        "INSERT INTO device_tool_recovery_batches (device_id, state, deadline, created_at) VALUES (?, 'open', CAST(strftime('%s','now') AS INTEGER) + ?, CAST(strftime('%s','now') AS INTEGER))",
    )
    .bind(device_id)
    .bind(deadline_secs)
    .execute(&mut *tx)
    .await?
    .last_insert_rowid();
    sqlx::query(
        "DELETE FROM device_tool_recovery_batches WHERE device_id=? AND id NOT IN (SELECT id FROM device_tool_recovery_batches WHERE device_id=? ORDER BY id DESC LIMIT ?)",
    )
    .bind(device_id)
    .bind(device_id)
    .bind(MAX_BATCHES_PER_DEVICE)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    let batch = load(database, id).await?;
    Ok(batch)
}

/// Records one complete walk as a member of the Device's open batch.
///
/// Returns `None` when no batch is open (the walk is not part of any recovery, e.g. a late result).
pub async fn record(
    database: &SqlitePool,
    device_id: i64,
    member_key: &str,
    tools: &[DiscoveredTool],
) -> Result<Option<Resolution>, sqlx::Error> {
    let mut tx = database.begin().await?;
    let open = sqlx::query_as::<_, (i64, i64)>(
        "SELECT id, deadline FROM device_tool_recovery_batches WHERE device_id=? AND state='open' ORDER BY id DESC LIMIT 1",
    )
    .bind(device_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((batch_id, deadline)) = open else {
        tx.commit().await?;
        return Ok(None);
    };

    let expired = sqlx::query_scalar::<_, i64>("SELECT CAST(strftime('%s','now') AS INTEGER) > ?")
        .bind(deadline)
        .fetch_one(&mut *tx)
        .await?
        != 0;
    if expired {
        sqlx::query("UPDATE device_tool_recovery_batches SET state='expired', completed_at=CAST(strftime('%s','now') AS INTEGER) WHERE id=?")
            .bind(batch_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        return Ok(Some(Resolution {
            state: BatchState::Expired,
            changed: false,
        }));
    }

    sqlx::query(
        "INSERT INTO device_tool_recovery_members (batch_id, member_key, tools, observed_at) VALUES (?, ?, ?, CAST(strftime('%s','now') AS INTEGER)) ON CONFLICT(batch_id, member_key) DO UPDATE SET tools=excluded.tools, observed_at=excluded.observed_at",
    )
    .bind(batch_id)
    .bind(member_key)
    .bind(encode(tools))
    .execute(&mut *tx)
    .await?;

    let members = sqlx::query_as::<_, (String, String)>(
        "SELECT member_key, tools FROM device_tool_recovery_members WHERE batch_id=?",
    )
    .bind(batch_id)
    .fetch_all(&mut *tx)
    .await?;

    let state = if (members.len() as i64) < REQUIRED_MEMBERS {
        Resolution {
            state: BatchState::Open,
            changed: false,
        }
    } else {
        let agreed = fingerprints(&members[0].1);
        let consistent = members
            .iter()
            .all(|(_, encoded)| fingerprints(encoded) == agreed);
        let blocked = sqlx::query_scalar::<_, String>(
            "SELECT original_name FROM device_tool_observations WHERE device_id=? AND blocked=1",
        )
        .bind(device_id)
        .fetch_all(&mut *tx)
        .await?;
        let complete = blocked.iter().all(|name| agreed.contains_key(name));
        if consistent && complete {
            apply(&mut tx, device_id, batch_id, &members[0].1).await?
        } else {
            Resolution {
                state: BatchState::Failed,
                changed: false,
            }
        }
    };

    if state.state != BatchState::Open {
        sqlx::query("UPDATE device_tool_recovery_batches SET state=?, completed_at=CAST(strftime('%s','now') AS INTEGER) WHERE id=?")
            .bind(state.state.as_str())
            .bind(batch_id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(Some(state))
}

/// Adopts the agreed contract for the Device, superseding any changed evidence under bounded
/// retention.  Returns whether a contract changed (and so existing approvals no longer match).
async fn apply(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    device_id: i64,
    batch_id: i64,
    agreed: &str,
) -> Result<Resolution, sqlx::Error> {
    let device_revision = sqlx::query_scalar::<_, i64>("SELECT revision FROM devices WHERE id=?")
        .bind(device_id)
        .fetch_optional(&mut **tx)
        .await?
        .unwrap_or_default();
    let tools: BTreeMap<String, Value> = serde_json::from_str(agreed).unwrap_or_default();
    let mut changed = false;
    for (name, value) in tools {
        let fingerprint = value
            .get("fingerprint")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let description = value
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let schema = value
            .get("input_schema")
            .cloned()
            .unwrap_or(Value::Null)
            .to_string();
        let existing = sqlx::query_as::<_, (String, i64)>(
            "SELECT fingerprint, observed_at FROM device_tool_observations WHERE device_id=? AND original_name=?",
        )
        .bind(device_id)
        .bind(&name)
        .fetch_optional(&mut **tx)
        .await?;
        match existing {
            Some((stored, observed_at)) if stored != fingerprint => {
                sqlx::query(
                    "INSERT INTO device_tool_observation_history (device_id, original_name, fingerprint, observed_at, superseded_at, batch_id) VALUES (?, ?, ?, ?, CAST(strftime('%s','now') AS INTEGER), ?)",
                )
                .bind(device_id)
                .bind(&name)
                .bind(&stored)
                .bind(observed_at)
                .bind(batch_id)
                .execute(&mut **tx)
                .await?;
                sqlx::query(
                    "UPDATE device_tool_observations SET description=?, input_schema=?, fingerprint=?, device_revision=?, blocked=0, revision=revision+1, observed_at=CAST(strftime('%s','now') AS INTEGER) WHERE device_id=? AND original_name=?",
                )
                .bind(&description)
                .bind(&schema)
                .bind(&fingerprint)
                .bind(device_revision)
                .bind(device_id)
                .bind(&name)
                .execute(&mut **tx)
                .await?;
                changed = true;
            }
            Some(_) => {
                // The contract is unchanged: just lift the block and refresh the Device revision.
                sqlx::query(
                    "UPDATE device_tool_observations SET device_revision=?, blocked=0, observed_at=CAST(strftime('%s','now') AS INTEGER) WHERE device_id=? AND original_name=?",
                )
                .bind(device_revision)
                .bind(device_id)
                .bind(&name)
                .execute(&mut **tx)
                .await?;
            }
            None => {
                sqlx::query(
                    "INSERT INTO device_tool_observations (device_id, original_name, description, input_schema, fingerprint, device_revision, blocked, observed_at) VALUES (?, ?, ?, ?, ?, ?, 0, CAST(strftime('%s','now') AS INTEGER))",
                )
                .bind(device_id)
                .bind(&name)
                .bind(&description)
                .bind(&schema)
                .bind(&fingerprint)
                .bind(device_revision)
                .execute(&mut **tx)
                .await?;
                changed = true;
            }
        }
    }
    sqlx::query(
        "DELETE FROM device_tool_observation_history WHERE device_id=? AND id NOT IN (SELECT id FROM device_tool_observation_history WHERE device_id=? ORDER BY id DESC LIMIT ?)",
    )
    .bind(device_id)
    .bind(device_id)
    .bind(MAX_HISTORY_PER_DEVICE)
    .execute(&mut **tx)
    .await?;
    Ok(Resolution {
        state: BatchState::Reviewable,
        changed,
    })
}

/// Latest batch for a Device incarnation, if any.
pub async fn current(
    database: &SqlitePool,
    device_id: i64,
) -> Result<Option<RecoveryBatch>, sqlx::Error> {
    let id = sqlx::query_scalar::<_, i64>(
        "SELECT id FROM device_tool_recovery_batches WHERE device_id=? ORDER BY id DESC LIMIT 1",
    )
    .bind(device_id)
    .fetch_optional(database)
    .await?;
    match id {
        Some(id) => Ok(Some(load(database, id).await?)),
        None => Ok(None),
    }
}

async fn load(database: &SqlitePool, batch_id: i64) -> Result<RecoveryBatch, sqlx::Error> {
    let (id, state, deadline, created_at, completed_at) = sqlx::query_as::<_, (i64, String, i64, i64, Option<i64>)>(
        "SELECT id, state, deadline, created_at, completed_at FROM device_tool_recovery_batches WHERE id=?",
    )
    .bind(batch_id)
    .fetch_one(database)
    .await?;
    let members = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM device_tool_recovery_members WHERE batch_id=?",
    )
    .bind(id)
    .fetch_one(database)
    .await?;
    Ok(RecoveryBatch {
        id,
        state: BatchState::parse(&state),
        deadline,
        created_at,
        completed_at,
        members,
        required: REQUIRED_MEMBERS,
    })
}

/// Fails any open batch for a Device incarnation (its observing Session disconnected).  Returns
/// whether a batch was actually failed.  Safe-fails rather than resolving: an incomplete batch must
/// never clear a block.
pub async fn fail_open(database: &SqlitePool, device_id: i64) -> Result<bool, sqlx::Error> {
    let affected = sqlx::query(
        "UPDATE device_tool_recovery_batches SET state='failed', completed_at=CAST(strftime('%s','now') AS INTEGER) WHERE device_id=? AND state='open'",
    )
    .bind(device_id)
    .execute(database)
    .await?
    .rows_affected();
    Ok(affected > 0)
}

/// Expires every open batch whose deadline is before `now`, returning how many were expired.
pub async fn sweep_expired(database: &SqlitePool, now: i64) -> Result<u64, sqlx::Error> {
    let affected = sqlx::query(
        "UPDATE device_tool_recovery_batches SET state='expired', completed_at=CAST(strftime('%s','now') AS INTEGER) WHERE state='open' AND deadline < ?",
    )
    .bind(now)
    .execute(database)
    .await?
    .rows_affected();
    Ok(affected)
}

/// Latest recovery batch per Device for one Agent, keyed by public Device id, for the admin surface.
pub async fn latest_for_agent(
    database: &SqlitePool,
    agent_id: i64,
) -> Result<BTreeMap<String, RecoveryBatch>, sqlx::Error> {
    let rows = sqlx::query_as::<_, (String, i64, String, i64, i64, Option<i64>, i64)>(
        "SELECT d.device_id, b.id, b.state, b.deadline, b.created_at, b.completed_at, (SELECT COUNT(*) FROM device_tool_recovery_members m WHERE m.batch_id=b.id) FROM device_tool_recovery_batches b JOIN devices d ON d.id=b.device_id WHERE d.agent_id=? AND b.id=(SELECT MAX(b2.id) FROM device_tool_recovery_batches b2 WHERE b2.device_id=b.device_id)",
    )
    .bind(agent_id)
    .fetch_all(database)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(device_id, id, state, deadline, created_at, completed_at, members)| {
                (
                    device_id,
                    RecoveryBatch {
                        id,
                        state: BatchState::parse(&state),
                        deadline,
                        created_at,
                        completed_at,
                        members,
                        required: REQUIRED_MEMBERS,
                    },
                )
            },
        )
        .collect())
}
