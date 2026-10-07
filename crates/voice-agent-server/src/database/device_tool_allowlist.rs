//! Observed Device tool contracts are copied only after a complete validated discovery, and the
//! Agent's explicit approval for one Device incarnation is what admits them to dispatch.
use super::Database;
use crate::tools::device_mcp::{DiscoveredTool, contract_fingerprint};
use std::collections::HashMap;

/// Records one complete validated Device `tools/list` walk.
///
/// A tool whose contract changed is marked `blocked` rather than overwritten: no observation wins
/// by arriving last.  `blocked` is never cleared here, so a conflict stays visible until Ticket
/// 13's recovery batch resolves it.  Returns `(changed, conflicted)`, where `conflicted` tells the
/// caller to close the affected sessions.
pub async fn observe(
    database: &Database,
    device_id: i64,
    tools: &[DiscoveredTool],
) -> Result<(bool, bool), sqlx::Error> {
    let mut tx = database.pool().begin().await?;
    let device_revision = sqlx::query_scalar::<_, i64>("SELECT revision FROM devices WHERE id=?")
        .bind(device_id)
        .fetch_optional(&mut *tx)
        .await?
        .unwrap_or_default();
    let existing = sqlx::query_as::<_, (String, String, i64)>(
        "SELECT original_name, fingerprint, blocked FROM device_tool_observations WHERE device_id=?",
    )
    .bind(device_id)
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .map(|(name, fingerprint, blocked)| (name, (fingerprint, blocked != 0)))
    .collect::<HashMap<_, _>>();

    let mut conflicted = false;
    let mut changed = false;
    for tool in tools {
        let fingerprint = contract_fingerprint(tool);
        match existing.get(&tool.original_name) {
            None => {
                sqlx::query(
                    "INSERT INTO device_tool_observations (device_id, original_name, description, input_schema, fingerprint, device_revision, blocked, observed_at) VALUES (?, ?, ?, ?, ?, ?, 0, strftime('%s','now'))",
                )
                .bind(device_id)
                .bind(&tool.original_name)
                .bind(&tool.description)
                .bind(tool.input_schema.to_string())
                .bind(&fingerprint)
                .bind(device_revision)
                .execute(&mut *tx)
                .await?;
                changed = true;
            }
            Some((stored, _)) if stored != &fingerprint => {
                // Conflicting valid observation: keep the original evidence, block the tool.
                sqlx::query(
                    "UPDATE device_tool_observations SET blocked=1, revision=revision+1, observed_at=strftime('%s','now') WHERE device_id=? AND original_name=?",
                )
                .bind(device_id)
                .bind(&tool.original_name)
                .execute(&mut *tx)
                .await?;
                conflicted = true;
                changed = true;
            }
            Some(_) => {
                // Same contract: refresh the observed Device revision without clearing `blocked`.
                sqlx::query(
                    "UPDATE device_tool_observations SET device_revision=?, observed_at=strftime('%s','now') WHERE device_id=? AND original_name=?",
                )
                .bind(device_revision)
                .bind(device_id)
                .bind(&tool.original_name)
                .execute(&mut *tx)
                .await?;
                changed = true;
            }
        }
    }
    tx.commit().await?;
    Ok((changed, conflicted))
}

/// Admission-time snapshot for one Device incarnation: whether the Agent participates in review,
/// and the exact approved `original_name -> fingerprint` set.  A non-participating Agent keeps the
/// legacy Device allowlist behavior, so it carries no reviewed contracts.
pub async fn load_admitted(
    database: &Database,
    agent_id: i64,
    device_id: i64,
) -> Result<(bool, HashMap<String, String>), sqlx::Error> {
    let mode =
        sqlx::query_scalar::<_, String>("SELECT mode FROM agent_speaker_policies WHERE agent_id=?")
            .bind(agent_id)
            .fetch_optional(database.pool())
            .await?
            .unwrap_or_else(|| "off".into());
    let participating = mode != "off";
    let contracts = if participating {
        sqlx::query_as::<_, (String, String)>(
            "SELECT a.original_name, a.fingerprint FROM agent_device_tool_allowlist a JOIN device_tool_observations o ON o.device_id=a.device_id AND o.original_name=a.original_name JOIN devices d ON d.id=a.device_id WHERE a.agent_id=? AND a.device_id=? AND a.allowed=1 AND a.sensitive=0 AND o.blocked=0 AND o.fingerprint=a.fingerprint AND o.device_revision=d.revision AND d.enabled=1",
        )
        .bind(agent_id)
        .bind(device_id)
        .fetch_all(database.pool())
        .await?
        .into_iter()
        .collect()
    } else {
        HashMap::new()
    };
    Ok((participating, contracts))
}
