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

struct Binding {
    device: i64,
    identity: String,
    client: String,
    revision: i64,
    origin: String,
    expires: Instant,
}
struct Tickets {
    pending: HashMap<[u8; 32], Binding>,
    issued: Vec<Instant>,
    sessions: Vec<Weak<CancellationToken>>,
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
    _permit: OwnedSemaphorePermit,
}
impl WebAssistantAuth {
    pub(crate) fn active(&self) -> usize {
        2 - self.capacity.available_permits()
    }
    pub(crate) fn invalidate(&self) {
        let mut store = self.tickets.lock().unwrap_or_else(|e| e.into_inner());
        store.pending.clear();
        for token in store.sessions.drain(..).filter_map(|t| t.upgrade()) {
            token.cancel();
        }
    }
    pub(crate) fn register_close(&self, token: &Arc<CancellationToken>) {
        let mut store = self.tickets.lock().unwrap_or_else(|e| e.into_inner());
        store.sessions.retain(|t| t.strong_count() != 0);
        store.sessions.push(Arc::downgrade(token));
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
    if super::cors::is_local_origin(origin) {
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
#[path = "web_assistant_auth_tests.rs"]
mod tests;
