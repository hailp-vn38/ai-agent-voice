//! Agent identification policy and candidate persistence.
use crate::database::{
    Database,
    agents::get_agent_by,
    audit::{AuditOutcome, audit, audit_conflict},
    writes::WriteError,
};
impl Database {
    pub(crate) async fn policy_state(&self, agent_id: i64) -> Result<(String, i64), sqlx::Error> {
        Ok(sqlx::query_as::<_, (String, i64)>(
            "SELECT mode,revision FROM agent_speaker_policies WHERE agent_id=?",
        )
        .bind(agent_id)
        .fetch_optional(&self.pool)
        .await?
        .unwrap_or_else(|| ("off".to_owned(), 1)))
    }
}
impl Database {
    pub(crate) async fn speaker_id(&self, key: &str) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar("SELECT id FROM speakers WHERE key=?")
            .bind(key)
            .fetch_one(&self.pool)
            .await
    }
}
impl Database {
    pub(crate) async fn put_policy(
        &self,
        key: &str,
        mode: &str,
        revision: i64,
        request_id: &str,
    ) -> Result<i64, WriteError> {
        let pool = &self.pool;
        let agent = match get_agent_by(self, key).await {
            Ok(agent) => agent,
            Err(cause) => {
                return Err(if matches!(cause, sqlx::Error::RowNotFound) {
                    WriteError::NotFound
                } else {
                    WriteError::Sql(cause)
                });
            }
        };
        let (_, current) = match self.policy_state(agent.id).await {
            Ok(value) => value,
            Err(cause) => return Err(WriteError::Sql(cause)),
        };
        if revision != current {
            audit_conflict(
                pool,
                request_id.to_owned(),
                "agent_speaker_policy",
                agent.id,
                revision,
            )
            .await;
            return Err(WriteError::Conflict("revision_conflict"));
        }
        let next = current + 1;
        let mut tx = match pool.begin().await {
            Ok(tx) => tx,
            Err(cause) => return Err(WriteError::Sql(cause)),
        };
        let result = sqlx::query(
            "INSERT INTO agent_speaker_policies (agent_id,mode,revision) VALUES (?,?,?) \
         ON CONFLICT(agent_id) DO UPDATE SET mode=excluded.mode,revision=excluded.revision \
         WHERE agent_speaker_policies.revision=?",
        )
        .bind(agent.id)
        .bind(mode)
        .bind(next)
        .bind(current)
        .execute(&mut *tx)
        .await;
        match result {
            Ok(result) if result.rows_affected() == 1 => {}
            Ok(_) => return Err(WriteError::Conflict("revision_conflict")),
            Err(cause) => return Err(WriteError::Sql(cause)),
        }
        if let Err(cause) = audit(
            &mut *tx,
            request_id,
            "agent_speaker_policy",
            Some(agent.id),
            "put",
            Some(current),
            Some(next),
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
        // Identification preferences take effect on the next connection.
        Ok(next)
    }
}
impl Database {
    pub(crate) async fn put_agent_speaker(
        &self,
        agent_key: &str,
        speaker_key: &str,
        revision: i64,
        max_candidates: usize,
        request_id: &str,
    ) -> Result<i64, WriteError> {
        let pool = &self.pool;
        let agent = match get_agent_by(self, agent_key).await {
            Ok(agent) => agent,
            Err(cause) => {
                return Err(if matches!(cause, sqlx::Error::RowNotFound) {
                    WriteError::NotFound
                } else {
                    WriteError::Sql(cause)
                });
            }
        };
        let speaker = match self.speaker_id(speaker_key).await {
            Ok(value) => value,
            Err(cause) => {
                return Err(if matches!(cause, sqlx::Error::RowNotFound) {
                    WriteError::NotFound
                } else {
                    WriteError::Sql(cause)
                });
            }
        };
        if agent.revision != revision {
            return Err(WriteError::Conflict("revision_conflict"));
        }
        let current: i64 = match sqlx::query_scalar(
            "SELECT COUNT(*) FROM agent_speaker_candidates WHERE agent_id=?",
        )
        .bind(agent.id)
        .fetch_one(pool)
        .await
        {
            Ok(value) => value,
            Err(cause) => return Err(WriteError::Sql(cause)),
        };
        let exists: i64 = match sqlx::query_scalar(
            "SELECT COUNT(*) FROM agent_speaker_candidates WHERE agent_id=? AND speaker_id=?",
        )
        .bind(agent.id)
        .bind(speaker)
        .fetch_one(pool)
        .await
        {
            Ok(value) => value,
            Err(cause) => return Err(WriteError::Sql(cause)),
        };
        if exists == 0 && current >= max_candidates as i64 {
            return Err(WriteError::Conflict("speaker_candidate_limit"));
        }
        let next = agent.revision + 1;
        let mut tx = match pool.begin().await {
            Ok(tx) => tx,
            Err(cause) => return Err(WriteError::Sql(cause)),
        };
        let inserted = sqlx::query(
            "INSERT INTO agent_speaker_candidates(agent_id,speaker_id,created_at) VALUES (?,?,?) \
         ON CONFLICT(agent_id,speaker_id) DO NOTHING",
        )
        .bind(agent.id)
        .bind(speaker)
        .bind(crate::database::unix_seconds().unwrap_or_default())
        .execute(&mut *tx)
        .await;
        if let Err(cause) = inserted {
            return Err(WriteError::Sql(cause));
        }
        let bumped =
            sqlx::query("UPDATE agents SET revision=?,updated_at=? WHERE id=? AND revision=?")
                .bind(next)
                .bind(crate::database::unix_seconds().unwrap_or_default())
                .bind(agent.id)
                .bind(revision)
                .execute(&mut *tx)
                .await;
        if !matches!(bumped, Ok(ref value) if value.rows_affected() == 1) {
            return Err(WriteError::Conflict("revision_conflict"));
        }
        if let Err(cause) = audit(
            &mut *tx,
            request_id,
            "agent_speaker",
            Some(agent.id),
            "put",
            Some(revision),
            Some(next),
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
        Ok(next)
    }
}
impl Database {
    pub(crate) async fn delete_agent_speaker(
        &self,
        agent_key: &str,
        speaker_key: &str,
        revision: i64,
        request_id: &str,
    ) -> Result<i64, WriteError> {
        let pool = &self.pool;
        let agent = match get_agent_by(self, agent_key).await {
            Ok(value) => value,
            Err(cause) => {
                return Err(if matches!(cause, sqlx::Error::RowNotFound) {
                    WriteError::NotFound
                } else {
                    WriteError::Sql(cause)
                });
            }
        };
        let speaker = match self.speaker_id(speaker_key).await {
            Ok(value) => value,
            Err(cause) => {
                return Err(if matches!(cause, sqlx::Error::RowNotFound) {
                    WriteError::NotFound
                } else {
                    WriteError::Sql(cause)
                });
            }
        };
        if revision != agent.revision {
            return Err(WriteError::Conflict("revision_conflict"));
        }
        let mut tx = match pool.begin().await {
            Ok(value) => value,
            Err(cause) => return Err(WriteError::Sql(cause)),
        };
        let deleted =
            sqlx::query("DELETE FROM agent_speaker_candidates WHERE agent_id=? AND speaker_id=?")
                .bind(agent.id)
                .bind(speaker)
                .execute(&mut *tx)
                .await;
        if !matches!(deleted, Ok(ref value) if value.rows_affected() == 1) {
            return Err(WriteError::NotFound);
        }
        let next = revision + 1;
        let bumped =
            sqlx::query("UPDATE agents SET revision=?,updated_at=? WHERE id=? AND revision=?")
                .bind(next)
                .bind(crate::database::unix_seconds().unwrap_or_default())
                .bind(agent.id)
                .bind(revision)
                .execute(&mut *tx)
                .await;
        if !matches!(bumped, Ok(ref value) if value.rows_affected() == 1) {
            return Err(WriteError::Conflict("revision_conflict"));
        }
        if let Err(cause) = audit(
            &mut *tx,
            request_id,
            "agent_speaker",
            Some(agent.id),
            "delete",
            Some(revision),
            Some(next),
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
        Ok(next)
    }
}
impl Database {
    pub(crate) async fn agent_speaker_candidates(
        &self,
        agent_id: i64,
        space: Option<String>,
        page: u32,
        page_size: u32,
    ) -> Result<Vec<(String, i64, i64, i64)>, sqlx::Error> {
        sqlx::query_as::<_, (String, i64, i64, i64)>(
            "SELECT s.key,s.enabled,EXISTS(SELECT 1 FROM speaker_voiceprints v \
            WHERE v.speaker_id=s.id AND v.embedding_space=?),COUNT(*) OVER() \
         FROM agent_speaker_candidates c JOIN speakers s ON s.id=c.speaker_id \
         WHERE c.agent_id=? ORDER BY s.key LIMIT ? OFFSET ?",
        )
        .bind(space)
        .bind(agent_id)
        .bind(page_size as i64)
        .bind(((page - 1) * page_size) as i64)
        .fetch_all(&self.pool)
        .await
    }
}
impl Database {
    pub(crate) async fn speaker_bindings(
        &self,
        speaker: i64,
        page: u32,
        page_size: u32,
    ) -> Result<Vec<(String, i64)>, sqlx::Error> {
        sqlx::query_as::<_, (String, i64)>(
            "SELECT a.key,COUNT(*) OVER() FROM agent_speaker_candidates c \
         JOIN agents a ON a.id=c.agent_id WHERE c.speaker_id=? \
         ORDER BY a.key LIMIT ? OFFSET ?",
        )
        .bind(speaker)
        .bind(page_size as i64)
        .bind(((page - 1) * page_size) as i64)
        .fetch_all(&self.pool)
        .await
    }
}
