//! Canonical projection of an Agent's exact candidate set, shared by calibration publication
//! (ticket 14) and Voice Session admission (ticket 16). Both sides must hash the *same* JSON, so
//! the projection lives here rather than in either caller.

use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;

/// Canonical JSON for an Agent's exact candidate set: candidate identities, voiceprint revisions
/// and spaces, per-Template grants, and the pinned preprocessing/audio/scoring contract.
pub async fn candidate_set(
    pool: &SqlitePool,
    agent_id: i64,
    agent_key: &str,
) -> Result<Value, sqlx::Error> {
    let candidates = sqlx::query_as::<_, (i64, String)>(
        "SELECT c.speaker_id, s.key FROM agent_speaker_candidates c \
         JOIN speakers s ON s.id=c.speaker_id WHERE c.agent_id=? ORDER BY s.key",
    )
    .bind(agent_id)
    .fetch_all(pool)
    .await?;
    let mut entries = Vec::with_capacity(candidates.len());
    for (speaker_id, speaker_key) in candidates {
        let voiceprints = sqlx::query_as::<_, (String, i64, String, i64, i64, String)>(
            "SELECT embedding_space,revision,provider_key,provider_revision,dims,\
             calibration_revision FROM speaker_voiceprints WHERE speaker_id=? \
             ORDER BY embedding_space",
        )
        .bind(speaker_id)
        .fetch_all(pool)
        .await?;
        let voiceprints = voiceprints
            .into_iter()
            .map(
                |(space, revision, provider_key, provider_revision, dimensions, calibration)| {
                    serde_json::json!({
                        "space": space,
                        "revision": revision,
                        "provider_key": provider_key,
                        "provider_revision": provider_revision,
                        "dimensions": dimensions,
                        "calibration_revision": calibration,
                    })
                },
            )
            .collect::<Vec<_>>();
        let grants = sqlx::query_scalar::<_, String>(
            "SELECT t.key FROM agent_speaker_template_grants g \
             JOIN agent_templates t ON t.id=g.template_id \
             WHERE g.agent_id=? AND g.speaker_id=? ORDER BY t.key",
        )
        .bind(agent_id)
        .bind(speaker_id)
        .fetch_all(pool)
        .await?;
        entries.push(serde_json::json!({
            "speaker": speaker_key,
            "templates": grants,
            "voiceprints": voiceprints,
        }));
    }
    Ok(serde_json::json!({
        "agent": agent_key,
        "candidates": entries,
        // ponytail: contract pinned as a constant; read live provider/audio descriptors if the
        // recognition pipeline (tickets 10–13) exposes them and drift becomes a real risk.
        "contract": {
            "preprocessing": "pcm16-mono16k-v1",
            "sample_rate": 16_000,
            "channels": 1,
            "bits_per_sample": 16,
            "scoring": "cosine",
        },
    }))
}

/// SHA-256 of the canonical JSON, used to match an admitted session against a calibration reload
/// that revoked exactly that set.
pub async fn candidate_set_digest(
    pool: &SqlitePool,
    agent_id: i64,
    agent_key: &str,
) -> Result<String, sqlx::Error> {
    let value = candidate_set(pool, agent_id, agent_key).await?;
    let json = serde_json::to_string(&value).expect("candidate set is JSON-serializable");
    Ok(hex(Sha256::digest(json.as_bytes())))
}

/// Lowercase hex, shared with calibration so both sides hash identically.
pub(crate) fn hex(bytes: impl AsRef<[u8]>) -> String {
    bytes
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
