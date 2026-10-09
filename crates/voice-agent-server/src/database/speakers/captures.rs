//! Quick capture reservation, promotion and voiceprint replacement transactions.
use super::publish_catalog_revision;
use crate::database::{
    Database,
    audit::{AuditOutcome, audit},
    writes::WriteError,
};
use uuid::Uuid;
const CAPTURE_TTL_SECONDS: i64 = 10 * 60;
const TOMBSTONE_SECONDS: i64 = 24 * 60 * 60;
#[derive(sqlx::FromRow)]
struct CaptureRow {
    runtime_id: String,
    embedding_space: String,
    dims: i64,
    vector: Option<Vec<u8>>,
    status: String,
    speaker_id: Option<i64>,
    expires_at: i64,
}

pub(crate) struct NewCapture<'a> {
    pub id: &'a str,
    pub runtime_id: &'a str,
    pub embedding_space: &'a str,
    pub dimension: usize,
    pub vector: Vec<u8>,
    pub created_at: i64,
    pub max_open: usize,
}
pub(crate) struct CapturePromotion<'a> {
    pub capture_id: &'a str,
    pub name: &'a str,
    pub description: Option<&'a str>,
    pub runtime_id: &'a str,
    pub embedding: Option<(&'a str, usize)>,
    pub max_speakers: usize,
}
pub(crate) struct CaptureCommit {
    pub key: String,
    pub speaker_id: i64,
    pub created: bool,
}
impl Database {
    pub(crate) async fn store_speaker_capture(
        &self,
        input: NewCapture<'_>,
    ) -> Result<(), WriteError> {
        let pool = &self.pool;
        let mut tx = match pool.begin().await {
            Ok(tx) => tx,
            Err(cause) => return Err(WriteError::Sql(cause)),
        };
        let active: i64 = match sqlx::query_scalar(
            "SELECT COUNT(*) FROM speaker_quick_captures WHERE status='accepted' AND expires_at>?",
        )
        .bind(input.created_at)
        .fetch_one(&mut *tx)
        .await
        {
            Ok(value) => value,
            Err(cause) => return Err(WriteError::Sql(cause)),
        };
        if active >= input.max_open as i64 {
            return Err(WriteError::Conflict("enrollment_quota_exceeded"));
        }
        let written = sqlx::query("INSERT INTO speaker_quick_captures (id,runtime_id,embedding_space,dims,vector,status,created_at,expires_at) VALUES (?,?,?,?,?,'accepted',?,?)")
        .bind(input.id)
        .bind(input.runtime_id)
        .bind(input.embedding_space)
        .bind(input.dimension as i64)
        .bind(input.vector)
        .bind(input.created_at)
        .bind(input.created_at + CAPTURE_TTL_SECONDS)
        .execute(&mut *tx).await;
        if written.is_err() || tx.commit().await.is_err() {
            return Err(WriteError::Unavailable);
        }
        Ok(())
    }
}
impl Database {
    pub(crate) async fn promote_speaker_capture(
        &self,
        input: CapturePromotion<'_>,
        request_id: &str,
    ) -> Result<CaptureCommit, WriteError> {
        let pool = &self.pool;
        let mut tx = match pool.begin().await {
            Ok(tx) => tx,
            Err(cause) => return Err(WriteError::Sql(cause)),
        };
        let capture = match sqlx::query_as::<_, CaptureRow>("SELECT runtime_id,embedding_space,dims,vector,status,speaker_id,expires_at FROM speaker_quick_captures WHERE id=?").bind(input.capture_id).fetch_optional(&mut *tx).await {
        Ok(Some(value)) => value, Ok(None) => return Err(WriteError::Missing("capture_not_found")), Err(cause) => return Err(WriteError::Sql(cause)),
    };
        if capture.status == "committed" {
            let speaker_id = capture.speaker_id.expect("committed capture has speaker");
            let speaker =
                match sqlx::query_as::<_, (String,)>("SELECT key FROM speakers WHERE id=?")
                    .bind(speaker_id)
                    .fetch_optional(&mut *tx)
                    .await
                {
                    Ok(Some(value)) => value.0,
                    _ => return Err(WriteError::Gone("capture_consumed")),
                };
            if tx.commit().await.is_err() {
                return Err(WriteError::Unavailable);
            }
            return Ok(CaptureCommit {
                key: speaker,
                speaker_id,
                created: false,
            });
        }
        if capture.status != "accepted"
            || capture.expires_at <= crate::database::unix_seconds().unwrap_or_default()
        {
            return Err(WriteError::Gone("capture_expired"));
        }
        if capture.runtime_id != input.runtime_id {
            return Err(WriteError::Conflict("capture_runtime_incompatible"));
        }
        // The captured vector must target the currently active built-in model.
        let Some((embedding_space, dimension)) = input.embedding else {
            return Err(WriteError::UnavailableCode("speaker_unavailable"));
        };
        if capture.embedding_space != embedding_space || capture.dims != dimension as i64 {
            return Err(WriteError::Conflict("speaker_embedding_space_changed"));
        }
        let count: i64 = match sqlx::query_scalar("SELECT COUNT(*) FROM speakers")
            .fetch_one(&mut *tx)
            .await
        {
            Ok(value) => value,
            Err(cause) => return Err(WriteError::Sql(cause)),
        };
        if count >= input.max_speakers as i64 {
            return Err(WriteError::Conflict("speaker_quota_exceeded"));
        }
        let key = format!("spk_{}", Uuid::new_v4().simple());
        let time = crate::database::unix_seconds().unwrap_or_default();
        // This no-op update obtains SQLite's write reservation without violating the committed-row
        // constraint before the new Speaker ID exists.
        let reserved = sqlx::query(
        "UPDATE speaker_quick_captures SET expires_at=expires_at WHERE id=? AND status='accepted'",
    )
    .bind(input.capture_id)
    .execute(&mut *tx)
    .await;
        if !matches!(reserved, Ok(ref result) if result.rows_affected() == 1) {
            return Err(WriteError::Conflict("capture_busy"));
        }
        let speaker_id = match sqlx::query("INSERT INTO speakers (key,name,description,enabled,revision,created_at,updated_at) VALUES (?,?,?,1,1,?,?)").bind(&key).bind(input.name).bind(input.description).bind(time).bind(time).execute(&mut *tx).await { Ok(result) => result.last_insert_rowid(), Err(cause) => return Err(WriteError::Mutation(cause)) };
        let vector = capture.vector.expect("accepted capture has vector");
        let voiceprint = sqlx::query(
        "INSERT INTO speaker_voiceprints (speaker_id,embedding_space,revision,sample_count,browser_validation_status,provider_id,provider_key,provider_revision,dims,vector,calibration_revision,enrolled_at,updated_at) VALUES (?,?,1,1,'passed',NULL,'builtin',1,?,?,'',?,?)"
    )
        .bind(speaker_id)
        .bind(&capture.embedding_space)
        .bind(capture.dims)
        .bind(vector)
        .bind(time)
        .bind(time)
        .execute(&mut *tx).await;
        let committed = sqlx::query("UPDATE speaker_quick_captures SET status='committed',vector=NULL,speaker_id=?,committed_at=?,expires_at=? WHERE id=? AND status='accepted'")
        .bind(speaker_id)
        .bind(time)
        .bind(time + TOMBSTONE_SECONDS)
        .bind(input.capture_id)
        .execute(&mut *tx)
        .await;
        if voiceprint.is_err()
            || !matches!(committed, Ok(ref result) if result.rows_affected() == 1)
            || publish_catalog_revision(&mut tx).await.is_err()
            || audit(
                &mut *tx,
                request_id,
                "speaker",
                Some(speaker_id),
                "quick_enrollment",
                None,
                Some(1),
                AuditOutcome::Success,
                1,
            )
            .await
            .is_err()
            || tx.commit().await.is_err()
        {
            return Err(WriteError::Unavailable);
        }
        Ok(CaptureCommit {
            key,
            speaker_id,
            created: true,
        })
    }
}
impl Database {
    pub(crate) async fn replace_speaker_voiceprint(
        &self,
        key: &str,
        capture_id: &str,
        expected_revision: i64,
        runtime: (&str, &str, usize),
        request_id: &str,
    ) -> Result<(), WriteError> {
        let (runtime_id, embedding_space, dimension) = runtime;
        let pool = &self.pool;
        let mut tx = match pool.begin().await {
            Ok(value) => value,
            Err(cause) => return Err(WriteError::Sql(cause)),
        };
        let speaker: Option<(i64, i64)> =
            match sqlx::query_as("SELECT id,revision FROM speakers WHERE key=?")
                .bind(key)
                .fetch_optional(&mut *tx)
                .await
            {
                Ok(value) => value,
                Err(cause) => return Err(WriteError::Sql(cause)),
            };
        let Some((speaker_id, revision)) = speaker else {
            return Err(WriteError::NotFound);
        };
        if expected_revision != revision {
            return Err(WriteError::Conflict("revision_conflict"));
        }
        let capture: Option<CaptureRow> = match sqlx::query_as(
        "SELECT runtime_id,embedding_space,dims,vector,status,speaker_id,expires_at FROM speaker_quick_captures WHERE id=?"
    ).bind(capture_id).fetch_optional(&mut *tx).await {
        Ok(value) => value, Err(cause) => return Err(WriteError::Sql(cause)),
    };
        let Some(capture) = capture else {
            return Err(WriteError::Missing("capture_not_found"));
        };
        if capture.status == "committed" {
            if capture.speaker_id == Some(speaker_id) {
                drop(tx);
                return Ok(());
            }
            return Err(WriteError::Gone("capture_consumed"));
        }
        if capture.status != "accepted"
            || capture.expires_at <= crate::database::unix_seconds().unwrap_or_default()
        {
            return Err(WriteError::Gone("capture_expired"));
        }
        if capture.runtime_id != runtime_id
            || capture.embedding_space != embedding_space
            || capture.dims != dimension as i64
        {
            return Err(WriteError::Conflict("speaker_embedding_space_changed"));
        }
        let now = crate::database::unix_seconds().unwrap_or_default();
        let reserved = sqlx::query(
        "UPDATE speaker_quick_captures SET expires_at=expires_at WHERE id=? AND status='accepted'",
    )
    .bind(capture_id)
    .execute(&mut *tx)
    .await;
        if !matches!(reserved, Ok(ref value) if value.rows_affected() == 1) {
            return Err(WriteError::Conflict("capture_busy"));
        }
        let bumped = sqlx::query(
            "UPDATE speakers SET revision=revision+1,updated_at=? WHERE id=? AND revision=?",
        )
        .bind(now)
        .bind(speaker_id)
        .bind(revision)
        .execute(&mut *tx)
        .await;
        if !matches!(bumped, Ok(ref value) if value.rows_affected() == 1) {
            return Err(WriteError::Conflict("revision_conflict"));
        }
        let updated = sqlx::query(
        "INSERT INTO speaker_voiceprints(speaker_id,embedding_space,revision,sample_count,browser_validation_status,provider_id,provider_key,provider_revision,dims,vector,calibration_revision,enrolled_at,updated_at) VALUES (?,?,1,1,'passed',NULL,'builtin',1,?,?,'',?,?) \
         ON CONFLICT(speaker_id,embedding_space) DO UPDATE SET \
         revision=speaker_voiceprints.revision+1,sample_count=1,browser_validation_status='passed', \
         provider_id=NULL,provider_key='builtin',provider_revision=1,dims=excluded.dims,vector=excluded.vector, \
         calibration_revision='',enrolled_at=excluded.enrolled_at,updated_at=excluded.updated_at"
    )
    .bind(speaker_id).bind(&capture.embedding_space)
    .bind(capture.dims).bind(capture.vector.expect("accepted capture has embedding"))
    .bind(now).bind(now).execute(&mut *tx).await;
        if let Err(cause) = updated {
            return Err(WriteError::Sql(cause));
        }
        let consumed = sqlx::query(
        "UPDATE speaker_quick_captures SET status='committed',vector=NULL,speaker_id=?,committed_at=?,expires_at=? WHERE id=? AND status='accepted'"
    )
    .bind(speaker_id).bind(now).bind(now + TOMBSTONE_SECONDS)
    .bind(capture_id).execute(&mut *tx).await;
        if !matches!(consumed, Ok(ref value) if value.rows_affected() == 1) {
            return Err(WriteError::Conflict("capture_busy"));
        }
        if let Err(cause) = publish_catalog_revision(&mut tx).await {
            return Err(WriteError::Sql(cause));
        }
        if let Err(cause) = audit(
            &mut *tx,
            request_id,
            "speaker",
            Some(speaker_id),
            "reenroll",
            Some(revision),
            Some(revision + 1),
            AuditOutcome::Success,
            1,
        )
        .await
        {
            return Err(WriteError::Sql(cause));
        }
        if let Err(cause) = tx.commit().await {
            return Err(WriteError::Sql(cause));
        }
        Ok(())
    }
}
