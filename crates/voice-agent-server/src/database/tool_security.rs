//! Shared admission/dispatch authority. Reviewed rights never expand an existing catalog.
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tokio_util::sync::CancellationToken;

/// One admitted session's cancellation token, held weakly so a gone session is dropped on the next
/// registration or invalidation instead of leaking.
type WeakCancellation = std::sync::Weak<CancellationToken>;

/// The exact Speaker dependencies one admitted Voice Session pinned at admission. Invalidation
/// matches on any one of them, so a session whose snapshot did not include the changed speaker is
/// left running (a valid addition never closes an old qualified snapshot).
#[derive(Clone, Debug)]
pub struct SpeakerDeps {
    pub agent: i64,
    pub template: i64,
    pub speakers: Vec<i64>,
    pub candidate_set_digest: Option<String>,
}

#[derive(Default, Debug)]
pub struct ToolSecurity {
    sessions: Mutex<Vec<(i64, Vec<i64>, WeakCancellation)>>,
    devices: Mutex<Vec<(i64, WeakCancellation)>>,
    speakers: Mutex<Vec<(SpeakerDeps, WeakCancellation)>>,
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
    /// Registers a Voice Session's pinned Speaker snapshot.  The returned token is cancelled by any
    /// later mutation that revokes one of these dependencies; the WebSocket then closes instead of
    /// keeping authority it no longer has.
    pub fn register_speaker(&self, deps: SpeakerDeps) -> Arc<CancellationToken> {
        let token = Arc::new(CancellationToken::new());
        let mut speakers = self.speakers.lock().unwrap_or_else(|e| e.into_inner());
        speakers.retain(|(_, token)| token.strong_count() != 0);
        speakers.push((deps, Arc::downgrade(&token)));
        token
    }
    /// Re-enrol, replace, disable or purge of a speaker: cancels every session that pinned it.
    pub fn invalidate_speaker(&self, speaker: i64) {
        self.cancel_speakers(|deps| deps.speakers.contains(&speaker));
    }
    /// Policy or grant changes for one Agent (unlink, grant reduction, policy mode change).
    pub fn invalidate_agent_speakers(&self, agent: i64) {
        self.cancel_speakers(|deps| deps.agent == agent);
    }
    /// Template update/disable: cancels every session admitted under that Template.
    pub fn invalidate_template_speakers(&self, template: i64) {
        self.cancel_speakers(|deps| deps.template == template);
    }
    /// Calibration reload that revoked one exact candidate set.
    pub fn invalidate_candidate_set(&self, digest: &str) {
        self.cancel_speakers(|deps| deps.candidate_set_digest.as_deref() == Some(digest));
    }
    fn cancel_speakers(&self, matches: impl Fn(&SpeakerDeps) -> bool) {
        for (_, token) in self
            .speakers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|(deps, _)| matches(deps))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn deps(agent: i64, template: i64, speakers: &[i64]) -> SpeakerDeps {
        SpeakerDeps {
            agent,
            template,
            speakers: speakers.to_vec(),
            candidate_set_digest: None,
        }
    }

    #[test]
    fn speaker_invalidation_matches_every_pinned_dependency() {
        let security = ToolSecurity::default();
        let by_agent = security.register_speaker(deps(1, 10, &[100]));
        let by_template = security.register_speaker(deps(2, 10, &[100]));
        let unrelated = security.register_speaker(deps(3, 11, &[101]));
        // Both pin speaker 100, so a speaker mutation closes both; the unrelated session survives.
        security.invalidate_speaker(100);
        assert!(by_agent.is_cancelled() && by_template.is_cancelled());
        assert!(!unrelated.is_cancelled());
        // A policy/grant change scopes to the Agent only.
        let agent_only = security.register_speaker(deps(1, 12, &[102]));
        assert!(!agent_only.is_cancelled());
        security.invalidate_agent_speakers(1);
        assert!(agent_only.is_cancelled());
        // A digest-scoped reload matches that exact set and nothing else.
        let mut pinned = deps(4, 13, &[103]);
        pinned.candidate_set_digest = Some("abc".into());
        let digest_pinned = security.register_speaker(pinned);
        let other = security.register_speaker(deps(4, 13, &[103]));
        security.invalidate_candidate_set("abc");
        assert!(digest_pinned.is_cancelled());
        assert!(!other.is_cancelled());
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
