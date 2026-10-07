//! Shared admission/dispatch authority. Reviewed rights never expand an existing catalog.
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tokio_util::sync::CancellationToken;

/// One admitted session's cancellation token, held weakly so a gone session is dropped on the next
/// registration or invalidation instead of leaking.
type WeakCancellation = std::sync::Weak<CancellationToken>;

#[derive(Default, Debug)]
pub struct ToolSecurity {
    sessions: Mutex<Vec<(i64, Vec<i64>, WeakCancellation)>>,
    devices: Mutex<Vec<(i64, WeakCancellation)>>,
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
    /// Registers a Voice Session admitted under one Device incarnation.  A conflicting discovery,
    /// a revoked approval or a re-enrolment cancels the returned token, which closes the affected
    /// WebSocket rather than letting it keep a stale catalog.
    pub fn register_device(&self, device: i64) -> Arc<CancellationToken> {
        let token = Arc::new(CancellationToken::new());
        let mut devices = self.devices.lock().unwrap_or_else(|e| e.into_inner());
        devices.retain(|(_, token)| token.strong_count() != 0);
        devices.push((device, Arc::downgrade(&token)));
        token
    }
    pub fn invalidate_device(&self, device: i64) {
        for (_, token) in self
            .devices
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|(id, _)| *id == device)
        {
            if let Some(token) = token.upgrade() {
                token.cancel();
            }
        }
    }
    pub fn invalidate_agent(&self, agent: i64) {
        for (_, _, token) in self
            .sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|(id, _, _)| *id == agent)
        {
            if let Some(token) = token.upgrade() {
                token.cancel();
            }
        }
    }
    pub fn invalidate_server(&self, server: i64) {
        for (_, _, token) in self
            .sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|(_, servers, _)| servers.contains(&server))
        {
            if let Some(token) = token.upgrade() {
                token.cancel();
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

/// The review state one admitted Device incarnation runs with.
///
/// `contracts` is the exact, approved `original_name -> fingerprint` snapshot admission resolved;
/// it never grows mid-session.  The Device keeps publishing its discovered contracts, but only
/// these can become LLM-visible, and a discovered fingerprint that disagrees with an approved one
/// is drift, not a new right.
#[derive(Clone, Debug)]
pub struct DeviceToolGuard {
    pub database: super::Database,
    pub agent_id: i64,
    pub device_id: i64,
    /// Whether the Agent participates in review at all.  A non-participating Agent keeps the
    /// legacy Device allowlist behavior, but its discoveries are still recorded as evidence.
    pub participating: bool,
    pub close: Arc<CancellationToken>,
    pub contracts: Arc<HashMap<String, String>>,
}
impl DeviceToolGuard {
    /// Re-checks the live approval immediately before dispatch: the same Device incarnation, the
    /// approved fingerprint, an unblocked observation at the current Device revision, and an
    /// allowlist entry that is still allowed and not sensitive.
    pub async fn allows(&self, original_name: &str) -> bool {
        if self.close.is_cancelled() {
            return false;
        }
        let Some(fingerprint) = self.contracts.get(original_name) else {
            return false;
        };
        let allowed = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM agent_device_tool_allowlist a JOIN device_tool_observations o ON o.device_id=a.device_id AND o.original_name=a.original_name JOIN devices d ON d.id=a.device_id WHERE a.agent_id=? AND a.device_id=? AND a.original_name=? AND a.fingerprint=? AND o.fingerprint=a.fingerprint AND o.device_revision=d.revision AND o.blocked=0 AND a.allowed=1 AND a.sensitive=0 AND d.enabled=1")
            .bind(self.agent_id).bind(self.device_id).bind(original_name).bind(fingerprint).fetch_one(self.database.pool()).await.unwrap_or(0) == 1;
        allowed && !self.close.is_cancelled()
    }
}
