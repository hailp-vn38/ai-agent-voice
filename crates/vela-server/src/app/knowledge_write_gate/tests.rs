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
        tokio::spawn(async move { reqwest::Client::new().put(write_url).send().await.unwrap() });
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
