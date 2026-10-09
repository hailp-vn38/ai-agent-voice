//! Immutable admission-time speaker facts; no inference or scoring.
use crate::audio::enrollment;
use crate::database::Database;
use crate::session::{ObserveCandidate, ObservePlan, ObserveResolution, SpeakerPolicyMode};
pub async fn resolve_speaker_policy(
    database: &Database,
    agent_id: i64,
) -> Result<Option<SpeakerPolicyMode>, sqlx::Error> {
    let mode: Option<String> =
        sqlx::query_scalar("SELECT mode FROM agent_speaker_policies WHERE agent_id = ?")
            .bind(agent_id)
            .fetch_optional(&database.pool)
            .await?;
    match mode {
        Some(raw) => SpeakerPolicyMode::parse(&raw)
            .map(Some)
            .ok_or_else(|| sqlx::Error::Protocol("unsupported speaker policy".into())),
        None => Ok(None),
    }
}

/// Resolve Agent-scoped candidates in the current built-in embedding space.
/// The Template parameter is retained temporarily for existing call sites, but
/// identification no longer depends on a Template or grant.
pub async fn resolve_observe_plan(
    database: &Database,
    agent_id: i64,
    _template_id: i64,
    embedding_space: &str,
) -> Result<ObserveResolution, sqlx::Error> {
    use sqlx::Row;
    let mode = resolve_speaker_policy(database, agent_id)
        .await?
        .unwrap_or(SpeakerPolicyMode::Off);
    if mode == SpeakerPolicyMode::Off {
        return Ok(ObserveResolution::Off);
    }
    let rows = sqlx::query(
        "SELECT s.id AS speaker_id,s.name AS key,v.vector AS vector,v.dims AS dims \
         FROM agent_speaker_candidates c \
         JOIN speakers s ON s.id=c.speaker_id AND s.enabled=1 \
         JOIN speaker_voiceprints v ON v.speaker_id=s.id AND v.embedding_space=? \
         WHERE c.agent_id=? ORDER BY s.id",
    )
    .bind(embedding_space)
    .bind(agent_id)
    .fetch_all(&database.pool)
    .await?;
    let candidates: Vec<ObserveCandidate> = rows
        .into_iter()
        .filter_map(|row| {
            let vector_bytes: Vec<u8> = row.get("vector");
            let dimension: i64 = row.get("dims");
            let vector = enrollment::decode_embedding(&vector_bytes);
            if dimension <= 0
                || vector.len() != dimension as usize
                || enrollment::validate_embedding(&vector, dimension as usize).is_err()
            {
                return None;
            }
            Some(ObserveCandidate {
                speaker_id: row.get("speaker_id"),
                // The identification-only label is the enrolled person's name.
                // It is not a credential and must never be used for authorization.
                key: row.get("key"),
                vector,
            })
        })
        .collect();
    if candidates.is_empty() {
        return Ok(ObserveResolution::Off);
    }
    let catalog_revision: i64 =
        sqlx::query_scalar("SELECT revision FROM speaker_catalog WHERE id=1")
            .fetch_one(&database.pool)
            .await?;
    Ok(ObserveResolution::Plan(ObservePlan {
        agent_id,
        template_id: 0,
        embedding_space: embedding_space.to_owned(),
        catalog_revision,
        policy: mode,
        candidates,
    }))
}
