//! Scoped one-use credentials and connection revocation for the System-owned Device.
use crate::database::devices::Device;
use ring::rand::{SecureRandom, SystemRandom};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, Weak},
    time::Duration,
};
use tokio::sync::{OwnedSemaphorePermit, RwLock, Semaphore};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

pub(crate) const REMINDER_TARGET_TTL: Duration = Duration::from_secs(15 * 60);
pub use crate::services::notifications::BoundReminderTarget;

struct Binding {
    device: i64,
    identity: String,
    client: String,
    revision: i64,
    origin: String,
    expires: Instant,
}
struct LiveSession {
    agent_key: String,
    close: Weak<CancellationToken>,
    reminder_target: Arc<Mutex<Option<BoundReminderTarget>>>,
}
struct Tickets {
    pending: HashMap<[u8; 32], Binding>,
    issued: Vec<Instant>,
    sessions: Vec<Weak<CancellationToken>>,
    live_sessions: HashMap<String, LiveSession>,
}

pub(crate) struct WebAssistantAuth {
    pub(crate) publication: RwLock<()>,
    tickets: Mutex<Tickets>,
    capacity: Arc<Semaphore>,
}
impl Default for WebAssistantAuth {
    fn default() -> Self {
        Self {
            publication: RwLock::new(()),
            tickets: Mutex::new(Tickets {
                pending: HashMap::new(),
                issued: Vec::new(),
                sessions: Vec::new(),
                live_sessions: HashMap::new(),
            }),
            capacity: Arc::new(Semaphore::new(2)),
        }
    }
}
#[derive(serde::Serialize)]
pub(crate) struct Ticket {
    device_id: String,
    client_id: String,
    ticket: String,
    expires_in_seconds: u8,
}
pub(crate) struct Connection {
    pub(crate) close: Arc<CancellationToken>,
    pub(crate) reminder_target: Arc<Mutex<Option<BoundReminderTarget>>>,
    _permit: OwnedSemaphorePermit,
}
impl WebAssistantAuth {
    pub(crate) fn active(&self) -> usize {
        2 - self.capacity.available_permits()
    }
    pub(crate) fn invalidate(&self) {
        let mut store = self.tickets.lock().unwrap_or_else(|e| e.into_inner());
        store.pending.clear();
        for (_, live) in store.live_sessions.drain() {
            if let Ok(mut guard) = live.reminder_target.lock() {
                *guard = None;
            }
        }
        for token in store.sessions.drain(..).filter_map(|t| t.upgrade()) {
            token.cancel();
        }
    }
    pub(crate) fn register_close(&self, token: &Arc<CancellationToken>) {
        let mut store = self.tickets.lock().unwrap_or_else(|e| e.into_inner());
        store.sessions.retain(|t| t.strong_count() != 0);
        store.sessions.push(Arc::downgrade(token));
    }
    pub(crate) fn register_session(
        &self,
        session_id: String,
        agent_key: String,
        connection: &Connection,
    ) {
        let mut store = self.tickets.lock().unwrap_or_else(|e| e.into_inner());
        store.live_sessions.retain(|_, s| {
            s.close
                .upgrade()
                .is_some_and(|token| !token.is_cancelled())
        });
        store.live_sessions.insert(
            session_id,
            LiveSession {
                agent_key,
                close: Arc::downgrade(&connection.close),
                reminder_target: Arc::clone(&connection.reminder_target),
            },
        );
    }
    pub(crate) fn session_agent_key(&self, session_id: &str) -> Option<String> {
        let mut store = self.tickets.lock().unwrap_or_else(|e| e.into_inner());
        store.live_sessions.retain(|_, s| {
            s.close
                .upgrade()
                .is_some_and(|token| !token.is_cancelled())
        });
        store.live_sessions.get(session_id).map(|s| s.agent_key.clone())
    }
    pub(crate) fn bind_reminder_target(
        &self,
        session_id: &str,
        expected_agent_key: &str,
        device_row_id: i64,
        device_id: String,
    ) -> Result<u64, &'static str> {
        let mut store = self.tickets.lock().unwrap_or_else(|e| e.into_inner());
        store.live_sessions.retain(|_, s| {
            s.close
                .upgrade()
                .is_some_and(|token| !token.is_cancelled())
        });
        let Some(live) = store.live_sessions.get(session_id) else {
            return Err("invalid_session");
        };
        if live.agent_key != expected_agent_key {
            return Err("unauthorized_target");
        }
        let mut guard = live
            .reminder_target
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        *guard = Some(BoundReminderTarget {
            device_row_id,
            device_id,
            expires_at: Instant::now() + REMINDER_TARGET_TTL,
        });
        Ok(REMINDER_TARGET_TTL.as_secs())
    }
    pub(crate) fn issue(&self, device: &Device, origin: &str) -> Result<Ticket, &'static str> {
        let now = Instant::now();
        let mut store = self.tickets.lock().unwrap_or_else(|e| e.into_inner());
        store.pending.retain(|_, b| b.expires > now);
        store
            .issued
            .retain(|t| now.duration_since(*t) < Duration::from_secs(60));
        if store.pending.len() >= 32 || store.issued.len() >= 20 {
            return Err("assistant_rate_limited");
        }
        let mut secret = [0u8; 32];
        SystemRandom::new()
            .fill(&mut secret)
            .map_err(|_| "assistant_ticket_unavailable")?;
        use base64::Engine;
        let ticket = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(secret);
        let client_id = format!("web_{}", uuid::Uuid::new_v4().simple());
        store.pending.insert(
            Sha256::digest(ticket.as_bytes()).into(),
            Binding {
                device: device.id,
                identity: device.device_id.clone(),
                client: client_id.clone(),
                revision: device.revision,
                origin: origin.to_owned(),
                expires: now + Duration::from_secs(30),
            },
        );
        store.issued.push(now);
        Ok(Ticket {
            device_id: device.device_id.clone(),
            client_id,
            ticket,
            expires_in_seconds: 30,
        })
    }
    pub(crate) fn consume(
        &self,
        ticket: &str,
        device: &Device,
        client: &str,
        origin: &str,
    ) -> Result<Connection, &'static str> {
        if ticket.len() != 43 {
            return Err("invalid_ticket");
        }
        let now = Instant::now();
        let mut store = self.tickets.lock().unwrap_or_else(|e| e.into_inner());
        store.pending.retain(|_, b| b.expires > now);
        let hash: [u8; 32] = Sha256::digest(ticket.as_bytes()).into();
        let binding = store.pending.remove(&hash).ok_or("invalid_ticket")?;
        if device.kind != "web_assistant"
            || device.enabled != 1
            || binding.device != device.id
            || binding.identity != device.device_id
            || binding.client != client
            || binding.revision != device.revision
            || binding.origin != origin
        {
            return Err("invalid_ticket");
        }
        let permit = self
            .capacity
            .clone()
            .try_acquire_owned()
            .map_err(|_| "assistant_capacity_exceeded")?;
        let close = Arc::new(CancellationToken::new());
        store.sessions.retain(|t| t.strong_count() != 0);
        store.sessions.push(Arc::downgrade(&close));
        Ok(Connection {
            close,
            reminder_target: Arc::new(Mutex::new(None)),
            _permit: permit,
        })
    }
}

pub(crate) fn trusted_origin(
    state: &super::AppState,
    headers: &axum::http::HeaderMap,
) -> Option<String> {
    if headers.get_all("origin").iter().count() != 1 {
        return None;
    }
    let origin = headers.get("origin")?;
    let text = origin.to_str().ok()?;
    if super::transport::cors::is_local_origin(origin) {
        return Some(text.to_owned());
    }
    let mut public = state.config.server.public_ws_url.clone();
    let scheme = if public.scheme() == "wss" {
        "https"
    } else {
        "http"
    };
    public.set_scheme(scheme).ok()?;
    (scheme == "https" && public.origin().ascii_serialization() == text).then(|| text.to_owned())
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
