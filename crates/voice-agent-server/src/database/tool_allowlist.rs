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
    ) -> Result<Option<ExternalToolGuard>, sqlx::Error> {
        let _publication = self.tool_security.publication.write().await;
        let mode = sqlx::query_scalar::<_, String>(
            "SELECT mode FROM agent_speaker_policies WHERE agent_id=?",
        )
        .bind(agent)
        .fetch_optional(self.pool())
        .await?
        .unwrap_or_else(|| "off".into());
        let participating = mode != "off";
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
                if !participating || allowed {
                    approved.push(tool.clone());
                }
                if participating && allowed {
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
        Ok(participating.then(|| ExternalToolGuard {
            database: self.clone(),
            agent_id: agent,
            close: self.tool_security.register(agent, ids),
            contracts: Arc::new(contracts),
        }))
    }
}
