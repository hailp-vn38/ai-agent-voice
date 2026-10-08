//! Explicit reload of the deployment calibration catalog and exact-candidate-set evidence.
//!
//! The catalog source is a fixed deployment path in `[speaker_recognition] calibration_source`;
//! reload never accepts a caller-supplied path, URL, or qualification field. A reload validates
//! the whole file before touching storage, so a bad file leaves the previous catalog published.
//! Evidence is keyed by `(calibration revision, candidate-set digest)`: publishing a new revision
//! or dropping an entry only invalidates the snapshots it actually covers, and the voiceprint
//! catalog generation is deliberately absent from the digest so a new generation does not revoke
//! qualification by itself.
//!
//! See ADR 0079 and ADR 0081, and the implementation guide §4.2, §6.2.

use super::*;
use crate::database::speaker_candidate_set::{candidate_set, candidate_set_digest, hex};
use axum::body::to_bytes;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;

/// A reload request carries no parameters; only an empty body or `{}` is accepted.
const MAX_RELOAD_BODY: usize = 1024;
const MAX_REVISION: usize = 128;
const MAX_SPACE: usize = 256;
const MAX_REPORT_REF: usize = 256;

/// The published calibration catalog, as validated from the fixed deployment source.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CalibrationFile {
    calibration_revision: String,
    #[serde(default)]
    profiles: Vec<CalibrationProfile>,
    #[serde(default)]
    evidence: Vec<EvidenceEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CalibrationProfile {
    space: String,
    status: String,
    #[serde(default)]
    report_ref: Option<String>,
    #[serde(default)]
    accept_threshold: Option<f64>,
    #[serde(default)]
    consistency_threshold: Option<f64>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct EvidenceEntry {
    agent_key: String,
    report_ref: String,
}

/// One validated evidence row, ready to publish.
struct EvidenceRow {
    candidate_set_digest: String,
    candidate_set_json: String,
    report_ref: String,
}

enum EvidenceError {
    UnknownAgent,
    Sql(sqlx::Error),
}

/// `POST /speaker-recognition/reload` — reload the fixed deployment calibration source.
pub(super) async fn reload(State(state): State<AppState>, request: Request) -> Response {
    let pool = match db(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let request = match reject_override(request).await {
        Ok(request) => request,
        Err(response) => return response,
    };
    let Some(path) = state.config.speaker_recognition.calibration_source.clone() else {
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "speaker_catalog_unavailable",
        );
    };
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(_) => {
            return error(
                &request,
                StatusCode::SERVICE_UNAVAILABLE,
                "speaker_catalog_unavailable",
            );
        }
    };
    let source_digest = hex(Sha256::digest(&bytes));
    let file: CalibrationFile = match serde_json::from_slice(&bytes) {
        Ok(file) => file,
        Err(_) => {
            return error(
                &request,
                StatusCode::UNPROCESSABLE_ENTITY,
                "calibration_invalid",
            );
        }
    };
    if let Err(code) = validate(&file) {
        return error(&request, StatusCode::UNPROCESSABLE_ENTITY, code);
    }
    let rows = match evidence_rows(pool, &file.evidence).await {
        Ok(rows) => rows,
        Err(EvidenceError::UnknownAgent) => {
            return error(
                &request,
                StatusCode::UNPROCESSABLE_ENTITY,
                "calibration_invalid",
            );
        }
        Err(EvidenceError::Sql(error_value)) => return sql_error(&request, &error_value),
    };

    let published_at = now();
    let profiles_json = match serde_json::to_string(&file.profiles) {
        Ok(value) => value,
        Err(_) => {
            return error(
                &request,
                StatusCode::UNPROCESSABLE_ENTITY,
                "calibration_invalid",
            );
        }
    };
    let revision = &file.calibration_revision;
    let mut transaction = match pool.begin().await {
        Ok(transaction) => transaction,
        Err(error_value) => return sql_error(&request, &error_value),
    };
    let existing = sqlx::query_scalar::<_, String>(
        "SELECT candidate_set_digest FROM speaker_calibration_evidence WHERE calibration_revision=?",
    )
    .bind(revision)
    .fetch_all(&mut *transaction)
    .await;
    let existing = match existing {
        Ok(existing) => existing,
        Err(error_value) => return sql_error(&request, &error_value),
    };
    let kept: Vec<&str> = rows
        .iter()
        .map(|row| row.candidate_set_digest.as_str())
        .collect();
    let revoked: Vec<String> = existing
        .into_iter()
        .filter(|digest| !kept.contains(&digest.as_str()))
        .collect();
    let publish = async {
        sqlx::query(
            "INSERT INTO speaker_calibration_catalog \
             (id,calibration_revision,source_digest,profiles_json,published_at) VALUES (1,?,?,?,?) \
             ON CONFLICT(id) DO UPDATE SET calibration_revision=excluded.calibration_revision, \
             source_digest=excluded.source_digest, profiles_json=excluded.profiles_json, \
             published_at=excluded.published_at",
        )
        .bind(revision)
        .bind(&source_digest)
        .bind(&profiles_json)
        .bind(published_at)
        .execute(&mut *transaction)
        .await?;
        for digest in &revoked {
            sqlx::query(
                "DELETE FROM speaker_calibration_evidence \
                 WHERE calibration_revision=? AND candidate_set_digest=?",
            )
            .bind(revision)
            .bind(digest)
            .execute(&mut *transaction)
            .await?;
        }
        for row in &rows {
            sqlx::query(
                "INSERT INTO speaker_calibration_evidence \
                 (calibration_revision,candidate_set_digest,candidate_set_json,report_ref,created_at) \
                 VALUES (?,?,?,?,?) ON CONFLICT(calibration_revision,candidate_set_digest) \
                 DO UPDATE SET candidate_set_json=excluded.candidate_set_json, \
                 report_ref=excluded.report_ref, created_at=excluded.created_at",
            )
            .bind(revision)
            .bind(&row.candidate_set_digest)
            .bind(&row.candidate_set_json)
            .bind(&row.report_ref)
            .bind(published_at)
            .execute(&mut *transaction)
            .await?;
        }
        Ok::<(), sqlx::Error>(())
    }
    .await;
    if let Err(error_value) = publish {
        return sql_error(&request, &error_value);
    }
    if let Err(error_value) = transaction.commit().await {
        return sql_error(&request, &error_value);
    }
    // The reload revoked exact candidate sets; close the sessions admitted under them so a stale
    // pass cannot restore evidence the catalog no longer carries. Kept sets stay live.
    if let Some(security) = security(&state) {
        for digest in &revoked {
            security.invalidate_candidate_set(digest);
        }
    }

    let evidence = rows
        .iter()
        .map(|row| {
            serde_json::json!({
                "candidate_set_digest": row.candidate_set_digest,
                "report_ref": row.report_ref,
            })
        })
        .collect::<Vec<_>>();
    Json(serde_json::json!({
        "calibration_revision": revision,
        "source_digest": source_digest,
        "profiles": file.profiles,
        "evidence": evidence,
        "revoked_candidate_sets": revoked,
    }))
    .into_response()
}

/// Accept only an empty body or `{}`; any qualification field, path, or URL override is refused.
#[allow(clippy::result_large_err)] // Handler callers return this HTTP response unchanged.
async fn reject_override(request: Request) -> Result<Request, Response> {
    let (parts, body) = request.into_parts();
    let bytes = match to_bytes(body, MAX_RELOAD_BODY).await {
        Ok(bytes) => bytes,
        Err(_) => {
            let request = Request::from_parts(parts, Body::empty());
            return Err(error(
                &request,
                StatusCode::PAYLOAD_TOO_LARGE,
                "payload_too_large",
            ));
        }
    };
    let request = Request::from_parts(parts, Body::empty());
    let blank = bytes.iter().all(u8::is_ascii_whitespace);
    if !blank {
        let empty_object =
            serde_json::from_slice::<serde_json::Map<String, serde_json::Value>>(&bytes)
                .is_ok_and(|object| object.is_empty());
        if !empty_object {
            return Err(error(
                &request,
                StatusCode::BAD_REQUEST,
                "calibration_reload_override_not_allowed",
            ));
        }
    }
    Ok(request)
}

/// Whole-catalog validation; every problem must be found before anything is published.
fn validate(file: &CalibrationFile) -> Result<(), &'static str> {
    if file.calibration_revision.is_empty() || file.calibration_revision.len() > MAX_REVISION {
        return Err("calibration_invalid");
    }
    for profile in &file.profiles {
        if profile.space.is_empty() || profile.space.len() > MAX_SPACE {
            return Err("calibration_invalid");
        }
        if !matches!(profile.status.as_str(), "preliminary" | "qualified") {
            return Err("calibration_invalid");
        }
        if profile.status == "qualified" && profile.report_ref.as_deref().is_none_or(str::is_empty)
        {
            return Err("calibration_invalid");
        }
        if profile
            .report_ref
            .as_deref()
            .is_some_and(|report_ref| report_ref.len() > MAX_REPORT_REF)
        {
            return Err("calibration_invalid");
        }
        for threshold in [profile.accept_threshold, profile.consistency_threshold]
            .into_iter()
            .flatten()
        {
            if !threshold.is_finite() || !(0.0..=1.0).contains(&threshold) {
                return Err("calibration_invalid");
            }
        }
    }
    let mut seen = std::collections::HashSet::new();
    for entry in &file.evidence {
        if entry.agent_key.is_empty()
            || entry.report_ref.is_empty()
            || entry.report_ref.len() > MAX_REPORT_REF
        {
            return Err("calibration_invalid");
        }
        if !seen.insert(entry.agent_key.as_str()) {
            return Err("calibration_invalid");
        }
    }
    Ok(())
}

/// Resolve every evidence entry to its agent's exact candidate set. Any missing Agent is a
/// validation failure, so the reload is all-or-nothing.
async fn evidence_rows(
    pool: &SqlitePool,
    entries: &[EvidenceEntry],
) -> Result<Vec<EvidenceRow>, EvidenceError> {
    let mut rows = Vec::with_capacity(entries.len());
    for entry in entries {
        let agent_id = sqlx::query_scalar::<_, i64>("SELECT id FROM agents WHERE key=?")
            .bind(&entry.agent_key)
            .fetch_optional(pool)
            .await
            .map_err(EvidenceError::Sql)?;
        let Some(agent_id) = agent_id else {
            return Err(EvidenceError::UnknownAgent);
        };
        let candidate_set = candidate_set(pool, agent_id, &entry.agent_key)
            .await
            .map_err(EvidenceError::Sql)?;
        let candidate_set_json = serde_json::to_string(&candidate_set).map_err(|_| {
            EvidenceError::Sql(sqlx::Error::Protocol(
                "candidate set is not serializable".into(),
            ))
        })?;
        rows.push(EvidenceRow {
            candidate_set_digest: hex(Sha256::digest(candidate_set_json.as_bytes())),
            candidate_set_json,
            report_ref: entry.report_ref.clone(),
        });
    }
    Ok(rows)
}

/// Whether the Agent's exact current candidate set is qualified under the published catalog.
pub(super) struct Qualification {
    pub(super) evidence: bool,
    pub(super) qualified_profile: bool,
}

impl Qualification {
    pub(super) fn is_qualified(&self) -> bool {
        self.evidence && self.qualified_profile
    }
}

pub(super) async fn qualification(
    pool: &SqlitePool,
    agent_id: i64,
    agent_key: &str,
) -> Result<Qualification, sqlx::Error> {
    let published = sqlx::query_as::<_, (String, String)>(
        "SELECT calibration_revision,profiles_json FROM speaker_calibration_catalog WHERE id=1",
    )
    .fetch_optional(pool)
    .await?;
    let Some((calibration_revision, profiles_json)) = published else {
        return Ok(Qualification {
            evidence: false,
            qualified_profile: false,
        });
    };
    let profiles: Vec<CalibrationProfile> =
        serde_json::from_str(&profiles_json).unwrap_or_default();
    let qualified_profile = profiles.iter().any(|profile| profile.status == "qualified");
    let digest = candidate_set_digest(pool, agent_id, agent_key).await?;
    let evidence = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM speaker_calibration_evidence \
         WHERE calibration_revision=? AND candidate_set_digest=?",
    )
    .bind(&calibration_revision)
    .bind(&digest)
    .fetch_one(pool)
    .await?
        > 0;
    Ok(Qualification {
        evidence,
        qualified_profile,
    })
}

/// Catalog status for the Admin summary.
pub(super) async fn status(pool: &SqlitePool) -> Result<serde_json::Value, sqlx::Error> {
    let published = sqlx::query_as::<_, (String, String, i64)>(
        "SELECT calibration_revision,profiles_json,published_at FROM speaker_calibration_catalog \
         WHERE id=1",
    )
    .fetch_optional(pool)
    .await?;
    let Some((calibration_revision, profiles_json, published_at)) = published else {
        return Ok(serde_json::json!({
            "revision": null,
            "status": "preliminary",
            "published_at": null,
            "profiles": [],
            "evidence_sets": 0,
        }));
    };
    let profiles: Vec<CalibrationProfile> =
        serde_json::from_str(&profiles_json).unwrap_or_default();
    let status = if profiles.iter().any(|profile| profile.status == "qualified") {
        "qualified"
    } else {
        "preliminary"
    };
    let evidence_sets = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM speaker_calibration_evidence WHERE calibration_revision=?",
    )
    .bind(&calibration_revision)
    .fetch_one(pool)
    .await?;
    Ok(serde_json::json!({
        "revision": calibration_revision,
        "status": status,
        "published_at": published_at,
        "profiles": profiles,
        "evidence_sets": evidence_sets,
    }))
}
