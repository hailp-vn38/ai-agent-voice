use super::*;
impl Database {
    pub(crate) async fn agent_templates(
        &self,
        agent_id: i64,
        page: u32,
        page_size: u32,
    ) -> Result<Vec<(String, String, String, i64, i64)>, sqlx::Error> {
        sqlx::query_as(
            "SELECT t.key,t.name,t.language,t.enabled,a.is_default \
         FROM agent_template_assignments a JOIN agent_templates t ON t.id=a.template_id \
         WHERE a.agent_id=? AND a.enabled=1 ORDER BY t.key LIMIT ? OFFSET ?",
        )
        .bind(agent_id)
        .bind(i64::from(page_size))
        .bind(i64::from((page - 1) * page_size))
        .fetch_all(&self.pool)
        .await
    }
    pub(crate) async fn template_agents(
        &self,
        template_id: i64,
        page: u32,
        page_size: u32,
    ) -> Result<(i64, Vec<(String, String, i64, i64)>), WriteError> {
        let pool = &self.pool;
        let total: i64 = match sqlx::query_scalar(
            "SELECT COUNT(*) FROM agent_template_assignments WHERE template_id=? AND enabled=1",
        )
        .bind(template_id)
        .fetch_one(pool)
        .await
        {
            Ok(value) => value,
            Err(error_value) => return Err(WriteError::Sql(error_value)),
        };
        let rows: Result<Vec<(String, String, i64, i64)>, _> = sqlx::query_as(
            "SELECT a.key,a.name,a.enabled,ata.is_default \
         FROM agent_template_assignments ata JOIN agents a ON a.id=ata.agent_id \
         WHERE ata.template_id=? AND ata.enabled=1 ORDER BY a.key LIMIT ? OFFSET ?",
        )
        .bind(template_id)
        .bind(i64::from(page_size))
        .bind(i64::from((page - 1) * page_size))
        .fetch_all(pool)
        .await;
        Ok((total, rows?))
    }
    pub(crate) async fn template_providers(
        &self,
        template_id: i64,
    ) -> Result<Vec<(String, String, i64)>, sqlx::Error> {
        sqlx::query_as(
            "SELECT b.provider_type,p.key,p.enabled FROM template_provider_bindings b \
         JOIN providers p ON p.id=b.provider_id WHERE b.template_id=? ORDER BY b.provider_type",
        )
        .bind(template_id)
        .fetch_all(&self.pool)
        .await
    }
    pub(crate) async fn unlink_template_provider(
        &self,
        key: &str,
        provider_type: &str,
        expected: i64,
        request_id: &str,
    ) -> Result<(), WriteError> {
        let pool = &self.pool;
        let template = match template_by(self, key).await {
            Ok(value) => value,
            Err(sqlx::Error::RowNotFound) => {
                return Err(WriteError::NotFound);
            }
            Err(error_value) => return Err(WriteError::Sql(error_value)),
        };
        if template.revision != expected {
            audit_conflict(pool, request_id.into(), "template", template.id, expected).await;
            return Err(WriteError::Conflict("revision_conflict"));
        }
        let mut tx = match pool.begin().await {
            Ok(value) => value,
            Err(_) => {
                return Err(WriteError::Unavailable);
            }
        };
        let deleted = match sqlx::query(
            "DELETE FROM template_provider_bindings WHERE template_id=? AND provider_type=?",
        )
        .bind(template.id)
        .bind(provider_type)
        .execute(&mut *tx)
        .await
        {
            Ok(result) => result.rows_affected() == 1,
            Err(error_value) => {
                let _ = tx.rollback().await;
                return Err(WriteError::Sql(error_value));
            }
        };
        if !deleted {
            let _ = tx.rollback().await;
            return Err(WriteError::NotFound);
        }
        let updated = match sqlx::query(
            "UPDATE agent_templates SET revision=revision+1,updated_at=? WHERE id=? AND revision=?",
        )
        .bind(crate::database::unix_seconds().unwrap_or_default())
        .bind(template.id)
        .bind(expected)
        .execute(&mut *tx)
        .await
        {
            Ok(result) => result.rows_affected() == 1,
            Err(error_value) => {
                let _ = tx.rollback().await;
                return Err(WriteError::Sql(error_value));
            }
        };
        if !updated {
            let _ = tx.rollback().await;
            audit_conflict(pool, request_id.into(), "template", template.id, expected).await;
            return Err(WriteError::Conflict("revision_conflict"));
        }
        if audit(
            &mut *tx,
            request_id,
            "template",
            Some(template.id),
            "unlink_provider",
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
    pub(crate) async fn unlink_agent_template(
        &self,
        agent_key: &str,
        template_key: &str,
        expected: i64,
        request_id: &str,
    ) -> Result<(), WriteError> {
        let pool = &self.pool;
        let agent = match get_agent_by(self, agent_key).await {
            Ok(value) => value,
            Err(sqlx::Error::RowNotFound) => {
                return Err(WriteError::NotFound);
            }
            Err(error_value) => return Err(WriteError::Sql(error_value)),
        };
        if agent.revision != expected {
            audit_conflict(pool, request_id.into(), "agent", agent.id, expected).await;
            return Err(WriteError::Conflict("revision_conflict"));
        }
        let assignment: Result<(i64,), _> = sqlx::query_as(
            "SELECT ata.id FROM agent_template_assignments ata \
         JOIN agent_templates t ON t.id=ata.template_id \
         WHERE ata.agent_id=? AND t.key=? AND ata.enabled=1",
        )
        .bind(agent.id)
        .bind(template_key)
        .fetch_one(pool)
        .await;
        let (assignment_id,) = match assignment {
            Ok(value) => value,
            Err(sqlx::Error::RowNotFound) => {
                return Err(WriteError::NotFound);
            }
            Err(error_value) => return Err(WriteError::Sql(error_value)),
        };
        let mut tx = match pool.begin().await {
            Ok(value) => value,
            Err(_) => {
                return Err(WriteError::Unavailable);
            }
        };
        let unlinked = match sqlx::query(
            "UPDATE agent_template_assignments SET enabled=0,is_default=0 WHERE id=? AND enabled=1",
        )
        .bind(assignment_id)
        .execute(&mut *tx)
        .await
        {
            Ok(result) => result.rows_affected() == 1,
            Err(error_value) => {
                let _ = tx.rollback().await;
                return Err(WriteError::Sql(error_value));
            }
        };
        let updated = if unlinked {
            match sqlx::query(
                "UPDATE agents SET revision=revision+1,updated_at=? WHERE id=? AND revision=?",
            )
            .bind(crate::database::unix_seconds().unwrap_or_default())
            .bind(agent.id)
            .bind(expected)
            .execute(&mut *tx)
            .await
            {
                Ok(result) => result.rows_affected() == 1,
                Err(error_value) => {
                    let _ = tx.rollback().await;
                    return Err(WriteError::Sql(error_value));
                }
            }
        } else {
            false
        };
        if !updated {
            let _ = tx.rollback().await;
            audit_conflict(pool, request_id.into(), "agent", agent.id, expected).await;
            return Err(WriteError::Conflict("revision_conflict"));
        }
        if audit(
            &mut *tx,
            request_id,
            "agent",
            Some(agent.id),
            "unlink_template",
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
}
