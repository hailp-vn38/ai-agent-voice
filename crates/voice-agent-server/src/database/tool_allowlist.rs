//! Observed external contracts are copied only after complete validated discovery.
use super::{AdmittedMcpServer, Database, tool_security::ExternalToolGuard};
use crate::tools::external_mcp::ResolvedExternalMcp;
use sha2::{Digest, Sha256};
use std::{collections::HashMap, sync::Arc};

impl Database {
    pub async fn block_external_observation(&self, server_id: i64) -> Result<(), sqlx::Error> {
        let _publication = self.tool_security.publication.write().await;
        let mut tx = self.pool().begin().await?;
        sqlx::query("UPDATE external_tool_observations SET blocked=1,revision=revision+1 WHERE server_id=? AND blocked=0").bind(server_id).execute(&mut *tx).await?;
        sqlx::query("UPDATE agent_external_tool_allowlist SET allowed=0,revision=revision+1 WHERE server_id=? AND allowed=1").bind(server_id).execute(&mut *tx).await?;
        tx.commit().await?;
        self.tool_security.invalidate_server(server_id);
        Ok(())
    }

    pub async fn review_external_catalog(
        &self,
        agent: i64,
        desired: &[AdmittedMcpServer],
        resolved: &mut [ResolvedExternalMcp],
    ) -> Result<ExternalToolGuard, sqlx::Error> {
        let _publication = self.tool_security.publication.write().await;
        let mut contracts = HashMap::new();
        let mut ids = Vec::new();
        for server in resolved {
            let Some(source) = desired.iter().find(|row| row.key == server.server_key) else {
                continue;
            };
            ids.push(source.id);
            let mut tx = self.pool().begin().await?;
            // Configuration revision is checked in the observation transaction: discovery may
            // finish after a config mutation; that old result must never become reviewable.
            let current =
                sqlx::query_scalar::<_, i64>("SELECT revision FROM mcp_servers WHERE id=?")
                    .bind(source.id)
                    .fetch_optional(&mut *tx)
                    .await?;
            if current != Some(source.revision) {
                server.tools = Arc::from([]);
                continue;
            }
            let mut invalidate = false;
            let mut approved = Vec::new();
            let names: Vec<&str> = server
                .tools
                .iter()
                .map(|tool| tool.original_name.as_str())
                .collect();
            let old_names = sqlx::query_scalar::<_, String>(
                "SELECT original_name FROM external_tool_observations WHERE server_id=?",
            )
            .bind(source.id)
            .fetch_all(&mut *tx)
            .await?;
            for name in old_names
                .iter()
                .filter(|name| !names.contains(&name.as_str()))
            {
                sqlx::query(
                    "DELETE FROM external_tool_observations WHERE server_id=? AND original_name=?",
                )
                .bind(source.id)
                .bind(name)
                .execute(&mut *tx)
                .await?;
                sqlx::query("UPDATE agent_external_tool_allowlist SET allowed=0,revision=revision+1 WHERE server_id=? AND original_name=?").bind(source.id).bind(name).execute(&mut *tx).await?;
                invalidate = true;
            }
            for tool in server.tools.iter() {
                let contract = serde_json::json!({"resource":source.id,"key":source.key,"endpoint":source.url,"transport":"streamable_http","auth_type":source.auth_type,"auth_header":source.auth_header_name,"auth_reference":source.secret_ref,"header_names":serde_json::from_str::<serde_json::Value>(&source.headers_json).unwrap_or_default().as_object().map(|m|m.keys().collect::<Vec<_>>()),"name":tool.original_name,"description":tool.description,"input_schema":tool.input_schema});
                let fingerprint = Sha256::digest(contract.to_string().as_bytes())
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>();
                let old = sqlx::query_scalar::<_, String>("SELECT fingerprint FROM external_tool_observations WHERE server_id=? AND original_name=?").bind(source.id).bind(&tool.original_name).fetch_optional(&mut *tx).await?;
                if old.as_ref().is_some_and(|old| old != &fingerprint) {
                    sqlx::query("UPDATE agent_external_tool_allowlist SET allowed=0,revision=revision+1 WHERE server_id=? AND original_name=?").bind(source.id).bind(&tool.original_name).execute(&mut *tx).await?;
                    invalidate = true;
                }
                sqlx::query("INSERT INTO external_tool_observations(server_id,original_name,description,input_schema,fingerprint,server_revision,observed_at) VALUES(?,?,?,?,?,?,unixepoch()) ON CONFLICT(server_id,original_name) DO UPDATE SET description=excluded.description,input_schema=excluded.input_schema,fingerprint=excluded.fingerprint,server_revision=excluded.server_revision,observed_at=excluded.observed_at,blocked=0,revision=revision+CASE WHEN blocked=1 OR fingerprint<>excluded.fingerprint OR server_revision<>excluded.server_revision THEN 1 ELSE 0 END")
                    .bind(source.id).bind(&tool.original_name).bind(&tool.description).bind(tool.input_schema.to_string()).bind(&fingerprint).bind(source.revision).execute(&mut *tx).await?;
                let allowed = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM agent_external_tool_allowlist WHERE agent_id=? AND server_id=? AND original_name=? AND fingerprint=? AND allowed=1 AND sensitive=0")
                    .bind(agent).bind(source.id).bind(&tool.original_name).bind(&fingerprint).fetch_one(&mut *tx).await? == 1;
                if allowed {
                    approved.push(tool.clone());
                }
                if allowed {
                    contracts.insert(
                        (source.key.clone(), tool.original_name.clone()),
                        (source.id, fingerprint),
                    );
                }
            }
            tx.commit().await?;
            if invalidate {
                self.tool_security.invalidate_server(source.id);
            }
            server.tools = Arc::from(approved);
        }
        Ok(ExternalToolGuard {
            database: self.clone(),
            agent_id: agent,
            close: self.tool_security.register(agent, ids),
            contracts: Arc::new(contracts),
        })
    }
}

use super::{
    audit::{AuditOutcome, audit},
    writes::WriteError,
};
use serde::Deserialize;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolReview {
    pub(crate) server_key: String,
    pub(crate) original_name: String,
    pub(crate) observed_revision: i64,
    pub(crate) fingerprint: String,
    #[serde(default)]
    pub(crate) allowed: bool,
    #[serde(default)]
    pub(crate) sensitive: bool,
}
impl Database {
    pub(crate) async fn external_tool_reviews(
        &self,
        agent_id: i64,
    ) -> Result<
        Vec<(
            String,
            String,
            String,
            String,
            String,
            i64,
            i64,
            i64,
            i64,
            i64,
            String,
        )>,
        sqlx::Error,
    > {
        sqlx::query_as::<_, (String,String,String,String,String,i64,i64,i64,i64,i64,String)>("SELECT m.key,o.original_name,o.description,o.input_schema,o.fingerprint,o.revision,o.observed_at,CASE WHEN a.fingerprint=o.fingerprint THEN COALESCE(a.allowed,0) ELSE 0 END,COALESCE(a.sensitive,0),COALESCE(a.revision,1),json_object('endpoint',m.url,'transport','streamable_http','auth_type',m.auth_type,'auth_header',m.auth_header_name,'auth_reference',CASE WHEN m.auth_type IN ('bearer','header') AND m.credential_json IS NULL THEN 'VOICE_MCP_'||upper(m.key)||'_TOKEN' ELSE NULL END) FROM agent_mcp_bindings b JOIN mcp_servers m ON m.id=b.mcp_server_id JOIN external_tool_observations o ON o.server_id=m.id AND o.server_revision=m.revision AND o.blocked=0 LEFT JOIN agent_external_tool_allowlist a ON a.agent_id=b.agent_id AND a.server_id=m.id AND a.original_name=o.original_name WHERE b.agent_id=? ORDER BY m.key,o.original_name").bind(agent_id).fetch_all(&self.pool).await
    }
    pub(crate) async fn review_external_tool(
        &self,
        key: &str,
        body: ToolReview,
        expected: i64,
        request_id: &str,
    ) -> Result<i64, WriteError> {
        let pool = &self.pool;
        let agent = match super::agents::get_agent_by(self, key).await {
            Ok(v) => v,
            Err(e) => return Err(WriteError::Sql(e)),
        };
        let security = &self.tool_security;
        let _publication = security.publication.write().await;
        let mut tx = match pool.begin().await {
            Ok(v) => v,
            Err(e) => return Err(WriteError::Sql(e)),
        };
        let observed=sqlx::query_scalar::<_,i64>("SELECT o.server_id FROM external_tool_observations o JOIN mcp_servers m ON m.id=o.server_id JOIN agent_mcp_bindings b ON b.mcp_server_id=m.id WHERE b.agent_id=? AND m.key=? AND o.original_name=? AND o.revision=? AND o.fingerprint=? AND o.server_revision=m.revision AND o.blocked=0").bind(agent.id).bind(&body.server_key).bind(&body.original_name).bind(body.observed_revision).bind(&body.fingerprint).fetch_optional(&mut *tx).await;
        let server = match observed {
            Ok(Some(v)) => v,
            Ok(None) => return Err(WriteError::Conflict("contract_conflict")),
            Err(e) => return Err(WriteError::Sql(e)),
        };
        let old=sqlx::query_as::<_,(i64,i64,i64,String)>("SELECT revision,allowed,sensitive,fingerprint FROM agent_external_tool_allowlist WHERE agent_id=? AND server_id=? AND original_name=?").bind(agent.id).bind(server).bind(&body.original_name).fetch_optional(&mut *tx).await;
        let old = match old {
            Ok(v) => v,
            Err(e) => return Err(WriteError::Sql(e)),
        };
        if old.as_ref().map(|v| v.0).unwrap_or(1) != expected {
            return Err(WriteError::Conflict("revision_conflict"));
        }
        let result=sqlx::query("INSERT INTO agent_external_tool_allowlist(agent_id,server_id,original_name,fingerprint,allowed,sensitive,revision) VALUES(?,?,?,?,?,?,2) ON CONFLICT(agent_id,server_id,original_name) DO UPDATE SET fingerprint=excluded.fingerprint,allowed=excluded.allowed,sensitive=excluded.sensitive,revision=revision+1")
        .bind(agent.id).bind(server).bind(&body.original_name).bind(&body.fingerprint).bind(i64::from(body.allowed)).bind(i64::from(body.sensitive)).execute(&mut *tx).await;
        if let Err(e) = result {
            return Err(WriteError::Sql(e));
        }
        if let Err(e) = audit(
            &mut *tx,
            request_id,
            "agent",
            Some(agent.id),
            "review_external_tool",
            Some(expected),
            Some(expected + 1),
            AuditOutcome::Success,
            1,
        )
        .await
        {
            return Err(WriteError::Sql(e));
        }
        if let Err(e) = tx.commit().await {
            return Err(WriteError::Sql(e));
        }
        if old.is_some_and(|(_, allowed, sensitive, fingerprint)| {
            allowed != 0
                && sensitive == 0
                && (!body.allowed || body.sensitive || fingerprint != body.fingerprint)
        }) {
            self.tool_security.invalidate_agent(agent.id);
        }
        Ok(expected + 1)
    }
}

impl Database {
    pub(super) async fn external_tool_allowed(
        &self,
        agent_id: i64,
        id: i64,
        name: &str,
        fingerprint: &str,
    ) -> Result<bool, sqlx::Error> {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM agent_external_tool_allowlist a JOIN external_tool_observations o ON o.server_id=a.server_id AND o.original_name=a.original_name JOIN mcp_servers m ON m.id=a.server_id JOIN agent_mcp_bindings b ON b.agent_id=a.agent_id AND b.mcp_server_id=a.server_id WHERE a.agent_id=? AND a.server_id=? AND a.original_name=? AND a.fingerprint=? AND o.fingerprint=a.fingerprint AND o.server_revision=m.revision AND o.blocked=0 AND a.allowed=1 AND a.sensitive=0 AND m.enabled=1 AND b.enabled=1")
            .bind(agent_id).bind(id).bind(name).bind(fingerprint).fetch_one(&self.pool).await.map(|count| count == 1)
    }
}
