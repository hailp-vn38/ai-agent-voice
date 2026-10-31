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

/// Upper bound on tracked jobs; the least recently used idle entry is evicted beyond this.
const MAX_JOB_ENTRIES: usize = 256;

#[derive(Clone)]
pub(crate) struct JobSession {
    /// Gateway account that owns the VM holding the session; reuse is only valid on this account.
    pub account_key: String,
    pub session_id: String,
    /// The previous call on this session gave up at its deadline, so Muse may still finish (or
    /// have finished) its video; the next call should look at the chat before asking again.
    pub last_timed_out: bool,
    client: Arc<MuseVmClient>,
}

impl JobSession {
    pub fn new(
        account_key: String,
        session_id: String,
        client: Arc<MuseVmClient>,
        last_timed_out: bool,
    ) -> Self {
        Self {
            account_key,
            session_id,
            last_timed_out,
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

/// Returns the entry for `key`, creating it on first use, and reaps expired idle jobs.
pub(crate) fn job_entry(key: &str) -> Arc<JobEntry> {
    let now = Instant::now();
    let mut expired: Vec<JobSession> = Vec::new();
    let entry = {
        let mut map = JOB_ENTRIES.lock().unwrap_or_else(|e| e.into_inner());
        map.retain(|_, entry| {
            if entry.idle_for(now) < JOB_SESSION_TTL {
                return true;
            }
            // A held gate means a call is still running: keep the entry.
            match entry.gate.try_lock() {
                Ok(mut slot) => {
                    expired.extend(slot.take());
                    false
                }
                Err(_) => true,
            }
        });
        if map.len() >= MAX_JOB_ENTRIES && !map.contains_key(key) {
            let oldest = map
                .iter()
                .filter(|(_, e)| e.gate.try_lock().is_ok())
                .max_by_key(|(_, e)| e.idle_for(now))
                .map(|(k, _)| k.clone());
            if let Some(oldest_key) = oldest
                && let Some(removed) = map.remove(&oldest_key)
                && let Ok(mut slot) = removed.gate.try_lock()
            {
                expired.extend(slot.take());
            }
        }
        Arc::clone(
            map.entry(key.to_string())
                .or_insert_with(|| Arc::new(JobEntry::new())),
        )
    };
    for session in expired {
        delete_in_background(&session);
    }
    entry
}

fn delete_in_background(session: &JobSession) {
    if !session.client.is_connected() {
        return;
    }
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        let client = Arc::clone(&session.client);
        let session_id = session.session_id.clone();
        handle.spawn(async move {
            let _ = client.delete_session(&session_id).await;
        });
    }
}

/// Releases and deletes the retained Muse session for a finished job (`key` = `root_session_id`).
pub(crate) async fn release_job_session(key: &str) {
    let removed = {
        let mut map = JOB_ENTRIES.lock().unwrap_or_else(|e| e.into_inner());
        map.remove(key)
    };
    let Some(entry) = removed else {
        return;
    };
    entry.set_account_hint(None);
    let mut slot = entry.lock().await;
    if let Some(session) = slot.take()
        && session.client.is_connected()
    {
        let _ = session.client.delete_session(&session.session_id).await;
    }
}
