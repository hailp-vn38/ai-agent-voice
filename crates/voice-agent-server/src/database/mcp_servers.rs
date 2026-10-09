//! MCP desired configuration and security-published binding mutations.
use super::{
    Database,
    agents::get_agent_by,
    audit::{AuditOutcome, audit, audit_conflict},
    writes::WriteError,
};
use sqlx::FromRow;
#[derive(FromRow)]
pub(crate) struct McpServerRow {
    pub(crate) id: i64,
    pub(crate) key: String,
    pub(crate) name: String,
    pub(crate) url: String,
    pub(crate) auth_type: String,
    pub(crate) auth_header_name: Option<String>,
    pub(crate) connect_timeout_ms: i64,
    pub(crate) request_timeout_ms: i64,
    pub(crate) enabled: i64,
    pub(crate) revision: i64,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
    pub(crate) credential_json: Option<String>,
}
pub(crate) async fn mcp_by(database: &Database, key: &str) -> Result<McpServerRow, sqlx::Error> {
    sqlx::query_as("SELECT id,key,name,url,auth_type,auth_header_name,connect_timeout_ms,request_timeout_ms,enabled,revision,created_at,updated_at,credential_json FROM mcp_servers WHERE key=?").bind(key).fetch_one(&database.pool).await
}

pub(crate) struct McpInput<'a> {
    pub key: &'a str,
    pub name: &'a str,
    pub url: &'a str,
    pub auth_type: &'a str,
    pub auth_header_name: Option<&'a str>,
    pub credential: Option<&'a str>,
    pub connect_timeout_ms: i64,
    pub request_timeout_ms: i64,
}
pub(crate) struct McpChanges<'a> {
    pub name: &'a str,
    pub url: &'a str,
    pub auth_type: &'a str,
    pub auth_header_name: Option<&'a str>,
    pub credential: Option<&'a str>,
    pub connect_timeout: i64,
    pub request_timeout: i64,
    pub enabled: i64,
}
impl Database {
    pub(crate) async fn list_mcp_servers(
        &self,
        page: u32,
        size: u32,
    ) -> Result<Vec<McpServerRow>, sqlx::Error> {
        sqlx::query_as::<_,McpServerRow>("SELECT id,key,name,url,auth_type,auth_header_name,connect_timeout_ms,request_timeout_ms,enabled,revision,created_at,updated_at,credential_json FROM mcp_servers ORDER BY key LIMIT ? OFFSET ?").bind(i64::from(size)).bind(i64::from((page-1)*size)).fetch_all(&self.pool).await
    }
    pub(crate) async fn agent_mcp_bindings(
        &self,
        agent_id: i64,
    ) -> Result<Vec<(String, i64, i64)>, sqlx::Error> {
        sqlx::query_as::<_,(String,i64,i64)>("SELECT m.key,b.enabled,b.required FROM agent_mcp_bindings b JOIN mcp_servers m ON m.id=b.mcp_server_id WHERE b.agent_id=? ORDER BY m.key").bind(agent_id).fetch_all(&self.pool).await
    }
    pub(crate) async fn create_mcp_server(
        &self,
        input: McpInput<'_>,
        request_id: &str,
    ) -> Result<(), WriteError> {
        let pool = &self.pool;
        let security = &self.tool_security;
        let _publication = security.publication.write().await;
        let mut tx = match pool.begin().await {
            Ok(v) => v,
            Err(_) => {
                return Err(WriteError::Unavailable);
            }
        };
        let result = sqlx::query("INSERT INTO mcp_servers(key,name,url,auth_type,auth_header_name,credential_json,connect_timeout_ms,request_timeout_ms,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?)").bind(input.key).bind(input.name).bind(input.url).bind(input.auth_type).bind(input.auth_header_name).bind(input.credential).bind(input.connect_timeout_ms).bind(input.request_timeout_ms).bind(crate::database::unix_seconds().unwrap_or_default()).bind(crate::database::unix_seconds().unwrap_or_default()).execute(&mut *tx).await;
        let resource_id = match result {
            Ok(v) => v.last_insert_rowid(),
            Err(e) => return Err(WriteError::Mutation(e)),
        };
        if audit(
            &mut *tx,
            request_id,
            "mcp_server",
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
    pub(crate) async fn update_mcp_server(
        &self,
        old: &McpServerRow,
        expected: i64,
        input: McpChanges<'_>,
        request_id: &str,
    ) -> Result<(), WriteError> {
        let pool = &self.pool;
        let security = &self.tool_security;
        let _publication = security.publication.write().await;
        let mut tx = match pool.begin().await {
            Ok(v) => v,
            Err(_) => {
                return Err(WriteError::Unavailable);
            }
        };
        let source_changed = old.url != input.url
            || old.auth_type != input.auth_type
            || old.auth_header_name.as_deref() != input.auth_header_name
            || old.enabled != input.enabled
            || old.credential_json.as_deref() != input.credential;
        let updated=sqlx::query("UPDATE mcp_servers SET name=?,url=?,auth_type=?,auth_header_name=?,credential_json=?,connect_timeout_ms=?,request_timeout_ms=?,enabled=?,revision=revision+1,updated_at=? WHERE id=? AND revision=?").bind(input.name).bind(input.url).bind(input.auth_type).bind(input.auth_header_name).bind(input.credential).bind(input.connect_timeout).bind(input.request_timeout).bind(input.enabled).bind(crate::database::unix_seconds().unwrap_or_default()).bind(old.id).bind(expected).execute(&mut *tx).await.map(|v|v.rows_affected()==1).unwrap_or(false);
        if !updated {
            let _ = tx.rollback().await;
            audit_conflict(pool, request_id.into(), "mcp_server", old.id, expected).await;
            return Err(WriteError::Conflict("revision_conflict"));
        };
        let approvals = if source_changed {
            sqlx::query("UPDATE agent_external_tool_allowlist SET allowed=0,revision=revision+1 WHERE server_id=?").bind(old.id).execute(&mut *tx).await
        } else {
            sqlx::query("UPDATE external_tool_observations SET server_revision=? WHERE server_id=?")
                .bind(expected + 1)
                .bind(old.id)
                .execute(&mut *tx)
                .await
        };
        if let Err(e) = approvals {
            return Err(WriteError::Sql(e));
        }
        if audit(
            &mut *tx,
            request_id,
            "mcp_server",
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
        };
        if source_changed {
            security.invalidate_server(old.id);
        }
        drop(_publication);
        Ok(())
    }
    pub(crate) async fn put_agent_mcp_binding(
        &self,
        key: &str,
        server_key: &str,
        enabled: bool,
        expected: i64,
        request_id: &str,
    ) -> Result<(), WriteError> {
        let pool = &self.pool;
        let agent = match get_agent_by(self, key).await {
            Ok(v) => v,
            Err(sqlx::Error::RowNotFound) => {
                return Err(WriteError::NotFound);
            }
            Err(e) => return Err(WriteError::Sql(e)),
        };
        if agent.revision != expected {
            audit_conflict(pool, request_id.into(), "agent", agent.id, expected).await;
            return Err(WriteError::Conflict("revision_conflict"));
        };
        let server = match mcp_by(self, server_key).await {
            Ok(v) => v,
            Err(sqlx::Error::RowNotFound) => {
                return Err(WriteError::Invalid("invalid_mcp_server"));
            }
            Err(e) => return Err(WriteError::Sql(e)),
        };
        let security = &self.tool_security;
        let _publication = security.publication.write().await;
        let mut tx = match pool.begin().await {
            Ok(v) => v,
            Err(_) => {
                return Err(WriteError::Unavailable);
            }
        };
        let ok=sqlx::query("INSERT INTO agent_mcp_bindings(agent_id,mcp_server_id,enabled,required,created_at) VALUES(?,?,?,?,?) ON CONFLICT(agent_id,mcp_server_id) DO UPDATE SET enabled=excluded.enabled,required=excluded.required").bind(agent.id).bind(server.id).bind(i64::from(enabled)).bind(0i64).bind(crate::database::unix_seconds().unwrap_or_default()).execute(&mut *tx).await.is_ok()&&sqlx::query("UPDATE agents SET revision=revision+1,updated_at=? WHERE id=? AND revision=?").bind(crate::database::unix_seconds().unwrap_or_default()).bind(agent.id).bind(expected).execute(&mut *tx).await.map(|v|v.rows_affected()==1).unwrap_or(false);
        if !ok {
            let _ = tx.rollback().await;
            return Err(WriteError::Conflict("revision_conflict"));
        };
        if audit(
            &mut *tx,
            request_id,
            "agent",
            Some(agent.id),
            "upsert_mcp_binding",
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
        };
        if !enabled {
            security.invalidate_agent(agent.id);
        }
        Ok(())
    }
    pub(crate) async fn unlink_agent_mcp_binding(
        &self,
        key: &str,
        server_key: &str,
        expected: i64,
        request_id: &str,
    ) -> Result<(), WriteError> {
        let pool = &self.pool;
        let agent = match get_agent_by(self, key).await {
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
        let security = &self.tool_security;
        let _publication = security.publication.write().await;
        let mut tx = match pool.begin().await {
            Ok(value) => value,
            Err(_) => {
                return Err(WriteError::Unavailable);
            }
        };
        let deleted = match sqlx::query(
        "DELETE FROM agent_mcp_bindings WHERE agent_id=? AND mcp_server_id=(SELECT id FROM mcp_servers WHERE key=?)",
    )
    .bind(agent.id)
    .bind(server_key)
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
            "unlink_mcp_binding",
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
        self.tool_security.invalidate_agent(agent.id);
        Ok(())
    }
    pub(crate) async fn mcp_conflict(&self, request_id: &str, id: i64, expected: i64) {
        audit_conflict(&self.pool, request_id.into(), "mcp_server", id, expected).await;
    }
}

#[cfg(test)]
mod tests;
