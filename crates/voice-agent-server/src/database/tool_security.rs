//! Shared admission/dispatch authority. Reviewed rights never expand an existing catalog.
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tokio_util::sync::CancellationToken;

#[derive(Default, Debug)]
pub struct ToolSecurity {
    sessions: Mutex<Vec<(i64, Vec<i64>, std::sync::Weak<CancellationToken>)>>,
    // ponytail: one deployment lock; split by resource if review traffic needs throughput.
    pub publication: Arc<tokio::sync::RwLock<()>>,
}
impl ToolSecurity {
    pub fn register(&self, agent: i64, servers: Vec<i64>) -> Arc<CancellationToken> {
        let token = Arc::new(CancellationToken::new());
        let mut sessions = self.sessions.lock().unwrap_or_else(|e| e.into_inner());
        sessions.retain(|(_, _, token)| token.strong_count() != 0);
        sessions.push((agent, servers, Arc::downgrade(&token)));
        token
    }
    pub fn invalidate_agent(&self, agent: i64) {
        for (id, _, token) in self
            .sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
        {
            if *id == agent {
                if let Some(token) = token.upgrade() {
                    token.cancel();
                }
            }
        }
    }
    pub fn invalidate_server(&self, server: i64) {
        for (_, servers, token) in self
            .sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
        {
            if servers.contains(&server) {
                if let Some(token) = token.upgrade() {
                    token.cancel();
                }
            }
        }
    }
}
#[derive(Clone, Debug)]
pub struct ExternalToolGuard {
    pub database: super::Database,
    pub agent_id: i64,
    pub close: Arc<CancellationToken>,
    pub contracts: Arc<HashMap<(String, String), (i64, String)>>,
}
impl ExternalToolGuard {
    pub async fn allows(&self, server: &str, name: &str) -> bool {
        if self.close.is_cancelled() {
            return false;
        }
        let Some((id, fingerprint)) = self.contracts.get(&(server.into(), name.into())) else {
            return false;
        };
        let allowed = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM agent_external_tool_allowlist a JOIN external_tool_observations o ON o.server_id=a.server_id AND o.original_name=a.original_name JOIN mcp_servers m ON m.id=a.server_id JOIN agent_mcp_bindings b ON b.agent_id=a.agent_id AND b.mcp_server_id=a.server_id WHERE a.agent_id=? AND a.server_id=? AND a.original_name=? AND a.fingerprint=? AND o.fingerprint=a.fingerprint AND o.server_revision=m.revision AND o.blocked=0 AND a.allowed=1 AND a.sensitive=0 AND m.enabled=1 AND b.enabled=1")
            .bind(self.agent_id).bind(id).bind(name).bind(fingerprint).fetch_one(self.database.pool()).await.unwrap_or(0) == 1;
        allowed && !self.close.is_cancelled()
    }
}
