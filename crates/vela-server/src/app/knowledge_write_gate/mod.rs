use axum::{
    extract::State,
    http::{Method, Request},
    middleware::Next,
    response::Response,
};
use std::time::Instant;
use tokio::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock};

/// Coordinates Knowledge writers with a whole-vault operation such as Git sync.
#[derive(Default)]
pub(crate) struct KnowledgeWriteGate {
    lock: Arc<RwLock<()>>,
    last_write: std::sync::Mutex<Option<Instant>>,
}

use std::sync::Arc;

impl KnowledgeWriteGate {
    pub(crate) async fn read(&self) -> OwnedRwLockReadGuard<()> {
        Arc::clone(&self.lock).read_owned().await
    }

    pub(crate) async fn write(&self) -> OwnedRwLockWriteGuard<()> {
        Arc::clone(&self.lock).write_owned().await
    }

    pub(crate) fn last_write(&self) -> Option<Instant> {
        self.last_write
            .lock()
            .ok()
            .and_then(|last_write| *last_write)
    }

    fn mark_write(&self) {
        if let Ok(mut last_write) = self.last_write.lock() {
            *last_write = Some(Instant::now());
        }
    }
}

pub(crate) async fn gate_knowledge_writes(
    State(gate): State<Arc<KnowledgeWriteGate>>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let mutating = request.uri().path().starts_with("/knowledge/")
        && matches!(
            *request.method(),
            Method::POST | Method::PUT | Method::PATCH | Method::DELETE
        );
    if !mutating {
        return next.run(request).await;
    }

    let _guard = gate.read().await;
    let response = next.run(request).await;
    gate.mark_write();
    response
}

#[cfg(test)]
mod tests;
