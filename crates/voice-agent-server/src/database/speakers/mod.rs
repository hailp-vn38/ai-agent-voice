//! Speaker profiles, voiceprint metadata and catalog revision publication.
use crate::database::{
    Database,
    audit::{AuditOutcome, audit, audit_conflict_action},
    writes::WriteError,
};
pub(crate) mod captures;
pub mod observations;
mod policy;
#[derive(sqlx::FromRow)]
pub(crate) struct SpeakerRow {
    pub(crate) id: i64,
    pub(crate) key: String,
    pub(crate) name: String,
    pub(crate) description: Option<String>,
    pub(crate) enabled: i64,
    pub(crate) revision: i64,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
}
#[derive(sqlx::FromRow)]
pub(crate) struct VoiceprintRow {
    pub(crate) revision: i64,
    pub(crate) sample_count: i64,
    pub(crate) embedding_space: String,
    pub(crate) provider_key: String,
    pub(crate) provider_revision: i64,
    pub(crate) browser_validation_status: String,
    pub(crate) enrolled_at: i64,
    pub(crate) calibration_revision: String,
}
pub(crate) async fn get_speaker_by(
    database: &Database,
    key: &str,
) -> Result<SpeakerRow, sqlx::Error> {
    sqlx::query_as::<_, SpeakerRow>(
        "SELECT id,key,name,description,enabled,revision,created_at,updated_at FROM speakers WHERE key=?",
    )
    .bind(key)
    .fetch_one(&database.pool)
    .await
}

pub(crate) async fn load_voiceprints(
    database: &Database,
    speaker_id: i64,
) -> Result<Vec<VoiceprintRow>, sqlx::Error> {
    sqlx::query_as::<_, VoiceprintRow>(
        "SELECT revision,sample_count,embedding_space,provider_key,provider_revision,browser_validation_status,enrolled_at,calibration_revision FROM speaker_voiceprints WHERE speaker_id=? ORDER BY embedding_space",
    )
    .bind(speaker_id)
    .fetch_all(&database.pool)
    .await
}

pub(super) async fn publish_catalog_revision(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar::<_, i64>(
        "UPDATE speaker_catalog SET revision=revision+1 WHERE id=1 RETURNING revision",
    )
    .fetch_one(&mut **tx)
    .await
}

pub(crate) struct SpeakerInput<'a> {
    pub key: &'a str,
    pub name: &'a str,
    pub description: Option<&'a str>,
}
pub(crate) struct SpeakerChanges<'a> {
    pub name: &'a str,
    pub description: Option<&'a str>,
    pub enabled: i64,
}
impl Database {
    pub(crate) async fn create_speaker(
        &self,
        input: SpeakerInput<'_>,
        max_speakers: usize,
        request_id: &str,
    ) -> Result<(), WriteError> {
        let pool = &self.pool;
        let count: i64 = match sqlx::query_scalar("SELECT COUNT(*) FROM speakers")
            .fetch_one(pool)
            .await
        {
            Ok(count) => count,
            Err(error_value) => return Err(WriteError::Sql(error_value)),
        };
        if count >= max_speakers as i64 {
            return Err(WriteError::Conflict("speaker_quota_exceeded"));
        }
        let time = crate::database::unix_seconds().unwrap_or_default();
        let mut tx = match pool.begin().await {
            Ok(tx) => tx,
            Err(_) => {
                return Err(WriteError::Unavailable);
            }
        };
        let result = sqlx::query(
        "INSERT INTO speakers (key,name,description,enabled,revision,created_at,updated_at) VALUES (?,?,?,1,1,?,?)",
    )
    .bind(input.key)
    .bind(input.name)
    .bind(input.description)
    .bind(time)
    .bind(time)
    .execute(&mut *tx)
    .await;
        let resource_id = match result {
            Ok(value) => value.last_insert_rowid(),
            Err(sqlx::Error::Database(error_value)) if error_value.is_unique_violation() => {
                return Err(WriteError::Conflict("speaker_key_conflict"));
            }
            Err(error_value) => return Err(WriteError::Mutation(error_value)),
        };
        if audit(
            &mut *tx,
            request_id,
            "speaker",
            Some(resource_id),
            "create",
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
        Ok(())
    }
    pub(crate) async fn update_speaker(
        &self,
        old: &SpeakerRow,
        expected: i64,
        input: SpeakerChanges<'_>,
        request_id: &str,
    ) -> Result<(), WriteError> {
        let pool = &self.pool;
        let mut tx = match pool.begin().await {
            Ok(tx) => tx,
            Err(_) => {
                return Err(WriteError::Unavailable);
            }
        };
        let update = sqlx::query(
        "UPDATE speakers SET name=?,description=?,enabled=?,revision=revision+1,updated_at=? WHERE id=? AND revision=?",
    )
    .bind(input.name)
    .bind(input.description)
    .bind(input.enabled)
    .bind(crate::database::unix_seconds().unwrap_or_default())
    .bind(old.id)
    .bind(expected)
    .execute(&mut *tx)
    .await;
        let updated = matches!(update, Ok(result) if result.rows_affected() == 1);
        if !updated {
            let _ = tx.rollback().await;
            audit_conflict_action(
                pool,
                request_id.to_owned(),
                "speaker",
                old.id,
                expected,
                "update",
            )
            .await;
            return Err(WriteError::Conflict("revision_conflict"));
        }
        // Disabling a speaker revokes its authority: publish the security invalidation.
        if input.enabled == 0
            && old.enabled != 0
            && publish_catalog_revision(&mut tx).await.is_err()
        {
            let _ = tx.rollback().await;
            return Err(WriteError::Unavailable);
        }
        if audit(
            &mut *tx,
            request_id,
            "speaker",
            Some(old.id),
            "update",
            Some(expected),
            Some(expected + 1),
            AuditOutcome::Success,
            1,
        )
        .await
        .is_err()
            || tx.commit().await.is_err()
        {
            return Err(WriteError::Unavailable);
        }
        Ok(())
    }
    pub(crate) async fn delete_speaker(
        &self,
        old: &SpeakerRow,
        expected: i64,
        request_id: &str,
    ) -> Result<(), WriteError> {
        let pool = &self.pool;
        let mut tx = match pool.begin().await {
            Ok(tx) => tx,
            Err(cause) => return Err(WriteError::Sql(cause)),
        };
        let in_use: i64 = match sqlx::query_scalar(
            "SELECT COUNT(*) FROM agent_speaker_candidates WHERE speaker_id=?",
        )
        .bind(old.id)
        .fetch_one(&mut *tx)
        .await
        {
            Ok(value) => value,
            Err(cause) => return Err(WriteError::Sql(cause)),
        };
        if in_use > 0 {
            return Err(WriteError::Conflict("speaker_in_use"));
        }
        // Committed captures require a non-null speaker_id; remove them before the FK sets it null.
        if let Err(cause) = sqlx::query("DELETE FROM speaker_quick_captures WHERE speaker_id=?")
            .bind(old.id)
            .execute(&mut *tx)
            .await
        {
            return Err(WriteError::Sql(cause));
        }
        let result = sqlx::query("DELETE FROM speakers WHERE id=? AND revision=?")
            .bind(old.id)
            .bind(expected)
            .execute(&mut *tx)
            .await;
        let deleted = match result {
            Ok(outcome) => outcome.rows_affected() == 1,
            Err(cause) => return Err(WriteError::Sql(cause)),
        };
        if !deleted {
            let _ = tx.rollback().await;
            return Err(WriteError::Conflict("revision_conflict"));
        }
        if let Err(cause) = publish_catalog_revision(&mut tx).await {
            return Err(WriteError::Sql(cause));
        }
        if audit(
            &mut *tx,
            request_id,
            "speaker",
            Some(old.id),
            "delete",
            Some(expected),
            None,
            AuditOutcome::Success,
            1,
        )
        .await
        .is_err()
            || tx.commit().await.is_err()
        {
            return Err(WriteError::Unavailable);
        }
        Ok(())
    }
    pub(crate) async fn purge_speaker_voiceprints(
        &self,
        old: &SpeakerRow,
        expected: i64,
        request_id: &str,
    ) -> Result<(), WriteError> {
        let pool = &self.pool;
        let mut tx = match pool.begin().await {
            Ok(tx) => tx,
            Err(_) => {
                return Err(WriteError::Unavailable);
            }
        };
        let result = async {
            sqlx::query("DELETE FROM speaker_voiceprints WHERE speaker_id=?")
                .bind(old.id)
                .execute(&mut *tx)
                .await?;
            sqlx::query("UPDATE speakers SET revision=revision+1 WHERE id=?")
                .bind(old.id)
                .execute(&mut *tx)
                .await?;
            publish_catalog_revision(&mut tx).await?;
            Ok::<(), sqlx::Error>(())
        }
        .await;
        if let Err(error_value) = result {
            let _ = tx.rollback().await;
            return Err(WriteError::Sql(error_value));
        }
        if audit(
            &mut *tx,
            request_id,
            "speaker",
            Some(old.id),
            "purge_voiceprint",
            Some(expected),
            Some(expected + 1),
            AuditOutcome::Success,
            1,
        )
        .await
        .is_err()
            || tx.commit().await.is_err()
        {
            return Err(WriteError::Unavailable);
        }
        Ok(())
    }
    pub(crate) async fn speaker_conflict(
        &self,
        request_id: &str,
        id: i64,
        expected: i64,
        action: &str,
    ) {
        audit_conflict_action(
            &self.pool,
            request_id.into(),
            "speaker",
            id,
            expected,
            action,
        )
        .await;
    }
    pub(crate) async fn list_speakers(
        &self,
        sort: Option<&str>,
        enabled: Option<bool>,
        enrollment_status: Option<&str>,
        page: u32,
        page_size: u32,
    ) -> Result<(i64, Vec<SpeakerRow>), WriteError> {
        let pool = &self.pool;
        let order = match sort.unwrap_or("key") {
            "key" => "key ASC",
            "-key" => "key DESC",
            "name" => "name ASC",
            "-name" => "name DESC",
            "updated_at" => "updated_at DESC, id DESC",
            "-updated_at" => "updated_at ASC, id ASC",
            "revision" => "revision DESC, id DESC",
            "-revision" => "revision ASC, id ASC",
            _ => return Err(WriteError::Invalid("invalid_query")),
        };
        let mut filters: Vec<String> = Vec::new();
        if let Some(enabled) = enabled {
            filters.push(format!("enabled = {}", i64::from(enabled)));
        }
        match enrollment_status {
            None => {}
            Some("enrolled") => filters.push(
                "EXISTS (SELECT 1 FROM speaker_voiceprints v WHERE v.speaker_id = speakers.id)"
                    .into(),
            ),
            Some("unenrolled") => filters.push(
                "NOT EXISTS (SELECT 1 FROM speaker_voiceprints v WHERE v.speaker_id = speakers.id)"
                    .into(),
            ),
            Some("draft") => return Err(WriteError::Invalid("invalid_query")),
            Some(_) => return Err(WriteError::Invalid("invalid_query")),
        }
        let where_clause = if filters.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", filters.join(" AND "))
        };
        let total = match sqlx::query_scalar::<_, i64>(&format!(
            "SELECT COUNT(*) FROM speakers{where_clause}"
        ))
        .fetch_one(pool)
        .await
        {
            Ok(total) => total,
            Err(error_value) => return Err(WriteError::Sql(error_value)),
        };
        let rows = sqlx::query_as::<_, SpeakerRow>(&format!(
        "SELECT id,key,name,description,enabled,revision,created_at,updated_at FROM speakers{where_clause} ORDER BY {order} LIMIT ? OFFSET ?"
    ))
    .bind(i64::from(page_size))
    .bind(i64::from((page - 1) * page_size))
    .fetch_all(pool)
    .await;
        let items = match rows {
            Ok(rows) => rows,
            Err(error_value) => return Err(WriteError::Sql(error_value)),
        };
        Ok((total, items))
    }
}
