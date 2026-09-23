use crate::{
    audio::VadSegmenterConfig,
    config::AppConfig,
    protocol::{ClientMessage, ServerHello, parse_client_message},
    providers::ProviderSet,
    session::{ActiveTurnLimiter, OutboundMessage, SessionActor, SessionEvent, SessionRuntimes},
    workers::{AsrWorkerRuntime, LlmRuntime, TtsWorkerRuntime, VadWorkerRuntime},
};
use axum::{
    Router,
    extract::{
        Query, State,
        ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, Request, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{sync::mpsc, time::timeout};
use tower_http::trace::TraceLayer;
use tracing::{debug, info, warn};
use uuid::Uuid;

mod ota;
mod state;
mod websocket;

pub use state::AppState;

/// Application seam for tests and other callers that have already initialized providers.
pub fn router_with_providers(config: AppConfig, providers: Arc<ProviderSet>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/voice/ota/", get(ota::handler).post(ota::handler).options(ota::options))
        .route("/voice/v1/", get(websocket::handler))
        .with_state(AppState::new(config, providers))
        // Never include query parameters here: browser compatibility may carry an auth token.
        .layer(
            TraceLayer::new_for_http().make_span_with(|request: &Request<_>| {
                tracing::info_span!("http_request", method = %request.method(), path = request.uri().path())
            }),
        )
}

/// Builds the public application only after local provider validation and warmup succeed.
pub fn application(config: AppConfig) -> Result<Router, crate::providers::ProviderLoadError> {
    let providers = Arc::new(ProviderSet::load(&config)?);
    Ok(router_with_providers(config, providers))
}

async fn health() -> &'static str {
    "ok"
}
