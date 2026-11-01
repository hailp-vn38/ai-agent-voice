//! Job-scoped Muse session continuity for media generation.
//!
//! A job (identified by its root session id) that calls `video_create` several times should keep
//! working inside ONE Muse chat session: Muse still holds the clips it already produced, so a
//! retry can repair or extend them instead of regenerating from scratch. This registry maps a
//! job's continuation key to that session and serialises operations on the same key.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use tokio::sync::{Mutex as AsyncMutex, MutexGuard};

use super::super::client::MuseVmClient;

/// A retained job session is dropped (and deleted on the VM) after this long without use.
pub(super) const JOB_SESSION_TTL: Duration = Duration::from_secs(60 * 60);

/// Soft limit for cached jobs. Entries holding remote state remain until cleanup is acknowledged.
const MAX_JOB_ENTRIES: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct VideoRequestSignature {
    pub prompt: String,
    pub duration_secs: u32,
}

#[derive(Clone)]
pub(crate) struct JobSession {
    /// Gateway account that owns the VM holding the session; reuse is only valid on this account.
    pub account_key: String,
    pub session_id: String,
    /// The previous call on this session gave up at its deadline, so Muse may still finish (or
    /// have finished) its video; the next call should look at the chat before asking again.
    pub last_timed_out: bool,
    /// Prompt and duration of the timed-out request (from `request_json`), so recovery only
    /// triggers when the retry is for the same request.
    pub timed_out_request: Option<VideoRequestSignature>,
    /// File paths of earlier batches that already completed before the previous call timed out.
    pub batch_paths: Vec<String>,
    client: Option<Arc<MuseVmClient>>,
}

impl JobSession {
    pub fn new(
        account_key: String,
        session_id: String,
        client: Option<Arc<MuseVmClient>>,
        last_timed_out: bool,
        timed_out_request: Option<VideoRequestSignature>,
        batch_paths: Vec<String>,
    ) -> Self {
        Self {
            account_key,
            session_id,
            last_timed_out,
            timed_out_request,
            batch_paths,
            client,
        }
    }
}

/// Per-job state. The async gate serialises media calls of one job; the hint lets the caller pin
/// the gateway account without waiting for the gate.
pub(crate) struct JobEntry {
    gate: AsyncMutex<Option<JobSession>>,
    account_hint: Mutex<Option<String>>,
    touched: Mutex<Instant>,
}

impl JobEntry {
    fn new() -> Self {
        Self {
            gate: AsyncMutex::new(None),
            account_hint: Mutex::new(None),
            touched: Mutex::new(Instant::now()),
        }
    }

    /// Waits until no other call of the same job is running, then returns the retained session.
    pub async fn lock(&self) -> MutexGuard<'_, Option<JobSession>> {
        self.touch();
        self.gate.lock().await
    }

    /// Gateway account that last held this job's session, if any.
    pub fn account_hint(&self) -> Option<String> {
        self.account_hint
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn set_account_hint(&self, account_key: Option<String>) {
        *self.account_hint.lock().unwrap_or_else(|e| e.into_inner()) = account_key;
    }

    pub fn touch(&self) {
        *self.touched.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
    }

    fn idle_for(&self, now: Instant) -> Duration {
        now.duration_since(*self.touched.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

static JOB_ENTRIES: LazyLock<Mutex<HashMap<String, Arc<JobEntry>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Returns the entry for a job. Durable cleanup is owned by the server sweep,
/// which can protect recoverable operations and record acknowledged deletions.
pub(crate) fn job_entry(key: &str) -> Arc<JobEntry> {
    let mut map = JOB_ENTRIES.lock().unwrap_or_else(|e| e.into_inner());
    if map.len() >= MAX_JOB_ENTRIES {
        map.retain(|_, entry| entry.gate.try_lock().map_or(true, |slot| slot.is_some()));
    }
    Arc::clone(
        map.entry(key.to_string())
            .or_insert_with(|| Arc::new(JobEntry::new())),
    )
}

/// Deletes idle sessions that have no recoverable work, returning acknowledged IDs.
pub(crate) async fn sweep_idle_job_sessions(
    protected: &std::collections::HashSet<String>,
) -> Vec<String> {
    let keys: Vec<String> = {
        let map = JOB_ENTRIES.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        map.iter()
            .filter(|(key, entry)| {
                !protected.contains(*key) && entry.idle_for(now) >= JOB_SESSION_TTL
            })
            .map(|(key, _)| key.clone())
            .collect()
    };
    let mut deleted = Vec::new();
    for key in keys {
        if let Some(id) = release_job_session_inner(&key, true).await {
            deleted.push(id);
        }
    }
    deleted
}

pub(crate) async fn restore_job_session(
    key: &str,
    account_key: String,
    session_id: String,
    last_timed_out: bool,
    timed_out_request: Option<VideoRequestSignature>,
    batch_paths: Vec<String>,
) {
    let entry = job_entry(key);
    entry.set_account_hint(Some(account_key.clone()));
    let mut slot = entry.lock().await;
    *slot = Some(JobSession::new(
        account_key,
        session_id,
        None,
        last_timed_out,
        timed_out_request,
        batch_paths,
    ));
}

pub(crate) async fn restore_job_sessions_from_operations(
    ops: &[orchestrator_agent_rs::ToolOperationSummary],
) {
    for summary in ops {
        if !summary.root_has_active_work && summary.operation.remote_deleted_at.is_some() {
            continue;
        }
        if summary.operation.remote_deleted_at.is_some() {
            continue;
        }
        let op = &summary.operation;
        let (Some(cont_key), Some(acc_key), Some(sess_id)) = (
            op.continuation_key.as_deref(),
            op.account_key.as_ref(),
            op.remote_session_id.as_ref(),
        ) else {
            continue;
        };
        let timed_out_req = op
            .request_json
            .as_deref()
            .and_then(|s| serde_json::from_str::<VideoRequestSignature>(s).ok());
        let batch_paths = op
            .batch_paths_json
            .as_deref()
            .and_then(|s| serde_json::from_str::<Vec<String>>(s).ok())
            .unwrap_or_default();
        let last_timed_out = op.status == orchestrator_agent_rs::ToolOperationStatus::Expired;
        restore_job_session(
            cont_key,
            acc_key.clone(),
            sess_id.clone(),
            last_timed_out,
            timed_out_req,
            batch_paths,
        )
        .await;
    }
}

/// Deletes the remote session, returning its ID only after Muse acknowledges deletion.
/// Failed cleanup remains registered so the periodic sweep can retry it.
pub(crate) async fn release_job_session(key: &str) -> Option<String> {
    release_job_session_inner(key, false).await
}

async fn release_job_session_inner(key: &str, idle_only: bool) -> Option<String> {
    let entry = {
        let map = JOB_ENTRIES.lock().unwrap_or_else(|e| e.into_inner());
        map.get(key).cloned()
    }?;
    let mut slot = entry.gate.lock().await;
    if idle_only && entry.idle_for(Instant::now()) < JOB_SESSION_TTL {
        return None;
    }
    let session = slot.as_ref()?;
    if !delete_remote_session(session).await {
        return None;
    }
    let session_id = session.session_id.clone();
    *slot = None;
    entry.set_account_hint(None);
    let mut map = JOB_ENTRIES.lock().unwrap_or_else(|e| e.into_inner());
    if map
        .get(key)
        .is_some_and(|current| Arc::ptr_eq(current, &entry))
    {
        map.remove(key);
    }
    Some(session_id)
}

/// Cleanup must address every recorded remote session, not only the latest
/// session retained under a job's continuation key.
pub(crate) async fn release_recorded_job_session(
    op: &orchestrator_agent_rs::ToolOperation,
) -> Option<String> {
    let sid = op.remote_session_id.as_ref()?;
    if let Some(key) = op.continuation_key.as_deref() {
        let entry = {
            JOB_ENTRIES
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(key)
                .cloned()
        };
        if let Some(entry) = entry {
            let matches = entry.gate.lock().await.as_ref().is_some_and(|session| {
                session.session_id == *sid && Some(&session.account_key) == op.account_key.as_ref()
            });
            if matches {
                return release_job_session(key).await;
            }
        }
    }
    let session = JobSession::new(
        op.account_key.clone()?,
        sid.clone(),
        None,
        false,
        None,
        Vec::new(),
    );
    delete_remote_session(&session).await.then(|| sid.clone())
}

async fn delete_remote_session(session: &JobSession) -> bool {
    if let Some(client) = &session.client
        && client.is_connected()
    {
        return client.delete_session(&session.session_id).await.is_ok();
    }
    // Restored sessions have no client. Reopen the owning account, including
    // accounts cooling down for new requests: cleanup does not generate media.
    let cached_pool = {
        let pools = super::GLOBAL_POOLS
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        pools
            .values()
            .find(|pool| {
                pool.gateway_account_key().as_deref() == Some(session.account_key.as_str())
            })
            .cloned()
    };
    let pool = match cached_pool {
        Some(pool) => pool,
        None => {
            let hub = crate::services::llm_gateway::SharedGatewayHub::global();
            let Ok(lease) = hub.acquire_account_for_polling(&session.account_key) else {
                return false;
            };
            if lease.vendor != crate::services::llm_gateway::GatewayVendor::Muse {
                return false;
            }
            super::SharedMusePool::for_gateway_lease(
                &reqwest::Client::new(),
                &lease.account_key,
                lease.access_token.expose(),
                None,
            )
        }
    };
    let Ok(client) = pool.ensure_account_client(0).await else {
        return false;
    };
    client.delete_session(&session.session_id).await.is_ok()
}
