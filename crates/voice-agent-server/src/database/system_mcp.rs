//! Bounded, secret-free read projections for System MCP.
use super::Database;
use serde_json::{Value, json};

impl Database {
    pub(crate) async fn system_mcp_caller_allowed(
        &self,
        device: i64,
        agent: i64,
    ) -> Result<bool, sqlx::Error> {
        sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM devices d JOIN agents a ON a.id=d.agent_id WHERE d.id=? AND a.id=? AND d.kind='web_assistant' AND d.enabled=1 AND a.enabled=1)")
            .bind(device).bind(agent).fetch_one(&self.pool).await
    }

    pub(crate) async fn system_mcp_bindings(&self, agent: i64) -> Result<Vec<Value>, sqlx::Error> {
        let rows = sqlx::query_as::<_, (String, bool, bool)>("SELECT m.key,b.enabled,m.enabled FROM agent_mcp_bindings b JOIN mcp_servers m ON m.id=b.mcp_server_id WHERE b.agent_id=? ORDER BY m.key LIMIT 101")
            .bind(agent).fetch_all(&self.pool).await?;
        Ok(rows.into_iter().map(|(key, enabled, server_enabled)| json!({"server_key":key,"enabled":enabled,"server_enabled":server_enabled})).collect())
    }

    pub(crate) async fn system_mcp_tools(
        &self,
        agent: i64,
        server: Option<&str>,
        page: u32,
    ) -> Result<Vec<Value>, sqlx::Error> {
        let rows = sqlx::query_as::<_, (String, String, String, i64, bool, bool, bool)>(
            "SELECT m.key,o.original_name,o.fingerprint,o.revision,
             (o.blocked=0 AND o.server_revision=m.revision),
             (COALESCE(a.allowed,0)=1 AND a.fingerprint=o.fingerprint AND o.blocked=0 AND o.server_revision=m.revision),
             COALESCE(a.sensitive,0)=1
             FROM agent_mcp_bindings b JOIN mcp_servers m ON m.id=b.mcp_server_id
             JOIN external_tool_observations o ON o.server_id=m.id
             LEFT JOIN agent_external_tool_allowlist a ON a.agent_id=b.agent_id AND a.server_id=m.id AND a.original_name=o.original_name
             WHERE b.agent_id=? AND (? IS NULL OR m.key=?) ORDER BY m.key,o.original_name LIMIT 20 OFFSET ?")
            .bind(agent).bind(server).bind(server).bind(i64::from(page - 1) * 20).fetch_all(&self.pool).await?;
        Ok(rows.into_iter().map(|(server_key, name, fingerprint, revision, current, allowed, sensitive)|
            json!({"server_key":server_key,"tool_name":name,"fingerprint":fingerprint,"observed_revision":revision,"current":current,"allowed":allowed,"sensitive":sensitive,"presence":"observed_only"})).collect())
    }
}
