//! Deployment ceiling seeds the singleton once; all later Studio Assistant grants come from DB.
use crate::database::{
    Database,
    audit::{AuditOutcome, audit, audit_conflict},
    writes::WriteError,
};
use crate::tools::system_mcp::SystemToolGrants;
use serde::Serialize;

#[derive(Serialize, sqlx::FromRow)]
pub(crate) struct GrantSettings {
    #[serde(flatten)]
    #[sqlx(flatten)]
    pub grants: SystemToolGrants,
    pub revision: i64,
}
impl Database {
    pub(crate) async fn system_mcp_grants(
        &self,
        ceiling: bool,
    ) -> Result<GrantSettings, sqlx::Error> {
        sqlx::query(
            "INSERT OR IGNORE INTO system_mcp_grants(id,system_read,mcp_read) VALUES(1,?,?)",
        )
        .bind(ceiling)
        .bind(ceiling)
        .execute(&self.pool)
        .await?;
        sqlx::query_as("SELECT system_read,mcp_read,mcp_discover,mcp_manage_request,revision FROM system_mcp_grants WHERE id=1")
            .fetch_one(&self.pool).await
    }
    pub(crate) async fn save_system_mcp_grants(
        &self,
        ceiling: bool,
        grants: SystemToolGrants,
        expected: i64,
        request: &str,
    ) -> Result<GrantSettings, WriteError> {
        let _publication = self.tool_security.publication.write().await;
        let old = self
            .system_mcp_grants(ceiling)
            .await
            .map_err(WriteError::Sql)?;
        let mut tx = self.pool.begin().await.map_err(WriteError::Sql)?;
        let updated = sqlx::query("UPDATE system_mcp_grants SET system_read=?,mcp_read=?,mcp_discover=?,mcp_manage_request=?,revision=revision+1 WHERE id=1 AND revision=?")
            .bind(grants.system_read).bind(grants.mcp_read).bind(grants.mcp_discover).bind(grants.mcp_manage_request).bind(expected).execute(&mut *tx).await.map_err(WriteError::Sql)?;
        if updated.rows_affected() != 1 {
            let _ = tx.rollback().await;
            audit_conflict(&self.pool, request.into(), "system_mcp", 1, expected).await;
            return Err(WriteError::Conflict("revision_conflict"));
        }
        audit(
            &mut *tx,
            request,
            "system_mcp",
            Some(1),
            "grant",
            Some(expected),
            Some(expected + 1),
            AuditOutcome::Success,
            1,
        )
        .await
        .map_err(WriteError::Sql)?;
        tx.commit().await.map_err(WriteError::Sql)?;
        if old.grants.system_read && !grants.system_read
            || old.grants.mcp_read && !grants.mcp_read
            || old.grants.mcp_discover && !grants.mcp_discover
            || old.grants.mcp_manage_request && !grants.mcp_manage_request
        {
            self.tool_security.invalidate_system_mcp();
        }
        Ok(GrantSettings {
            grants,
            revision: expected + 1,
        })
    }
    pub(crate) async fn system_mcp_token_allowed(
        &self,
        jti: &str,
        expires_at: i64,
    ) -> Result<bool, sqlx::Error> {
        let now = crate::database::unix_seconds().unwrap_or(i64::MAX);
        if expires_at <= now {
            return Ok(false);
        }
        sqlx::query_scalar(
            "SELECT NOT EXISTS(SELECT 1 FROM system_mcp_token_revocations WHERE jti=?)",
        )
        .bind(jti)
        .fetch_one(&self.pool)
        .await
    }
    pub(crate) async fn revoke_system_mcp_token(
        &self,
        jti: &str,
        expires_at: i64,
        request: &str,
    ) -> Result<(), WriteError> {
        let _publication = self.tool_security.publication.write().await;
        // A caller-supplied expiry cannot shorten revocation of a token still valid for an hour.
        let expires_at = expires_at.max(
            crate::database::unix_seconds()
                .unwrap_or(i64::MAX)
                .saturating_add(3600),
        );
        let mut tx = self.pool.begin().await.map_err(WriteError::Sql)?;
        sqlx::query("DELETE FROM system_mcp_token_revocations WHERE expires_at<=unixepoch()")
            .execute(&mut *tx)
            .await
            .map_err(WriteError::Sql)?;
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM system_mcp_token_revocations")
            .fetch_one(&mut *tx)
            .await
            .map_err(WriteError::Sql)?;
        if count >= 4096 {
            return Err(WriteError::UnavailableCode("revocation_capacity"));
        }
        sqlx::query("INSERT INTO system_mcp_token_revocations(jti,expires_at) VALUES(?,?) ON CONFLICT(jti) DO UPDATE SET expires_at=MAX(expires_at,excluded.expires_at)").bind(jti).bind(expires_at).execute(&mut *tx).await.map_err(WriteError::Sql)?;
        audit(
            &mut *tx,
            request,
            "system_mcp",
            Some(1),
            "revoke_token",
            None,
            None,
            AuditOutcome::Success,
            1,
        )
        .await
        .map_err(WriteError::Sql)?;
        tx.commit().await.map_err(WriteError::Sql)
    }
}
