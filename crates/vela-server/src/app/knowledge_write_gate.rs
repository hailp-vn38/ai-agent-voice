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
mod tests {
    use super::*;
    use axum::{Router, http::StatusCode, middleware, routing::get};
    use std::{sync::Arc, time::Duration};

    async fn ok() -> StatusCode {
        StatusCode::OK
    }

    fn app(gate: Arc<KnowledgeWriteGate>) -> Router {
        Router::new()
            .route("/knowledge/pages/{id}", get(ok).put(ok))
            .route_layer(middleware::from_fn_with_state(gate, gate_knowledge_writes))
    }

    async fn serve(gate: Arc<KnowledgeWriteGate>) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            axum::serve(listener, app(gate)).await.unwrap();
        });
        (format!("http://{address}"), handle)
    }

    #[tokio::test]
    async fn knowledge_write_waits_for_write_guard_but_read_does_not() {
        let gate = Arc::new(KnowledgeWriteGate::default());
        let (base, server) = serve(Arc::clone(&gate)).await;
        let write_guard = gate.write().await;

        let read = reqwest::get(format!("{base}/knowledge/pages/example"))
            .await
            .unwrap();
        assert_eq!(read.status(), StatusCode::OK);

        let write_url = format!("{base}/knowledge/pages/example");
        let mut write =
            tokio::spawn(
                async move { reqwest::Client::new().put(write_url).send().await.unwrap() },
            );
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut write)
                .await
                .is_err()
        );
        // A pending request has acquired no guard yet, so no write timestamp is recorded.
        assert!(!write.is_finished());

        drop(write_guard);
        assert_eq!(write.await.unwrap().status(), StatusCode::OK);
        assert!(gate.last_write().is_some());
        server.abort();
    }
}
