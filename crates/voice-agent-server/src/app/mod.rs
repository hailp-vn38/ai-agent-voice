use crate::{
    audio::VadSegmenterConfig,
    config::AppConfig,
    database::secrets::EnvSecretResolver,
    database::{Database, DatabaseError},
    protocol::{ClientMessage, ServerHello, parse_client_message},
    providers::{ProviderSet, materialize_database_providers},
    session::{
        ActiveTurnLimiter, OutboundMessage, SessionActor, SessionEvent, SessionRuntimes,
        WriterEvent, WriterTurnOutcome,
    },
    workers::{AsrWorkerRuntime, LlmRuntime, TtsWorkerRuntime, VadWorkerRuntime},
};
use axum::{
    Router,
    extract::{
        Query, State,
        ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, Request, StatusCode, header},
    middleware,
    response::{IntoResponse, Response},
    routing::get,
};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use std::{sync::Arc, time::Duration};
use thiserror::Error;
use tokio::{sync::mpsc, time::timeout};
use tokio_util::sync::CancellationToken;
use tower_http::trace::TraceLayer;
use tracing::{debug, info, warn};
use uuid::Uuid;

mod admin;
mod ota;
mod state;
mod vision;
mod websocket;

pub use state::AppState;

/// Application seam for tests and other callers that have already initialized providers.
pub fn router_with_providers(config: AppConfig, providers: Arc<ProviderSet>) -> Router {
    router_with_state(AppState::from_provider_set(config, providers))
}

/// Public startup seam for deterministic bootstrap tests. SQLite opens and migrates before the
/// application router exists; the injected providers keep model loading outside this seam.
pub async fn bootstrap_with_providers(
    config: AppConfig,
    providers: Arc<ProviderSet>,
) -> Result<Router, BootstrapError> {
    let database = Database::connect_if_enabled(&config.database).await?;
    Ok(router_with_state(
        AppState::from_provider_set_with_database(config, providers, database),
    ))
}
pub fn router_with_state(state: AppState) -> Router {
    let vision_enabled = state.config.vision.enabled;
    let vision_config = state.config.vision.clone();
    let router = Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route(
            "/voice/ota/",
            get(ota::handler).post(ota::handler).options(ota::options),
        )
        .route("/voice/v1/", get(websocket::handler));
    let router = if vision_enabled {
        router.route(
            "/mcp/vision/explain",
            get(vision::get_handler)
                .post(vision::post_handler)
                .options(vision::options_handler)
                .layer(vision::body_limit(
                    vision_config.max_image_bytes,
                    vision_config.max_question_bytes,
                ))
                .layer(middleware::map_response(vision::cors_response)),
        )
    } else {
        router
    };
    let router = if state.config.database.enabled && state.config.api.enabled {
        router.nest("/api/admin", admin::router(state.clone()))
    } else {
        router
    };
    router
        .with_state(state)
        // Never include query parameters here: browser compatibility may carry an auth token.
        .layer(
            TraceLayer::new_for_http().make_span_with(|request: &Request<_>| {
                tracing::info_span!("http_request", method = %request.method(), path = request.uri().path())
            }),
        )
}

/// Builds the public application only after local provider validation and warmup succeed.
pub fn application(config: AppConfig) -> Result<Router, crate::providers::ProviderLoadError> {
    let loaded = crate::providers::load_local(&config)?;
    Ok(router_with_state(AppState::new(config, loaded)))
}

/// Production startup orders the database dependency before provider initialization and listener
/// binding. Database failure is therefore always a pre-bind failure.
pub async fn startup(config: AppConfig) -> Result<Router, BootstrapError> {
    startup_with_shutdown(config, CancellationToken::new()).await
}

/// Production startup with an application-owned shutdown signal shared by the listener and
/// already-upgraded Voice Sessions.
pub async fn startup_with_shutdown(
    config: AppConfig,
    shutdown: CancellationToken,
) -> Result<Router, BootstrapError> {
    startup_with_shutdown_and_secret_resolver(config, shutdown, Arc::new(EnvSecretResolver)).await
}

/// Production startup seam for deployments that own secret resolution.  The runtime receives
/// only this abstraction; provider adapters never read a resolver backend directly.
pub async fn startup_with_shutdown_and_secret_resolver(
    config: AppConfig,
    shutdown: CancellationToken,
    secret_resolver: Arc<dyn crate::database::secrets::SecretResolver>,
) -> Result<Router, BootstrapError> {
    let database = Database::connect_if_enabled(&config.database).await?;
    let desired_rows = match &database {
        Some(database) => database.enabled_provider_rows().await?,
        None => Vec::new(),
    };
    let startup_config = config.clone();
    let startup_secret_resolver = Arc::clone(&secret_resolver);
    let (loaded, database_materialization) = tokio::task::spawn_blocking(move || {
        let mut local = crate::providers::load_local(&startup_config)?;
        let database_materialization = materialize_database_providers(
            &startup_config,
            desired_rows,
            startup_secret_resolver.as_ref(),
        );
        let mut database_materialization = database_materialization;
        for collision in
            local.extend_database_without_collisions(database_materialization.take_loaded())
        {
            database_materialization.mark_unavailable(
                &collision,
                crate::providers::DatabaseRuntimeFailure::Runtime,
            );
        }
        let database_runtime_snapshot = database_materialization.snapshot();
        Ok::<_, crate::providers::ProviderLoadError>((local, database_runtime_snapshot))
    })
    .await
    .map_err(|_| BootstrapError::Provider)?
    .map_err(|_| BootstrapError::Provider)?;
    Ok(router_with_state(
        AppState::new_with_database_runtime_snapshot_resolver_and_shutdown(
            config,
            loaded,
            database,
            Some(database_materialization),
            secret_resolver,
            shutdown,
        ),
    ))
}

async fn health() -> &'static str {
    "ok"
}

async fn ready() -> &'static str {
    "ready"
}

#[derive(Debug, Error)]
pub enum BootstrapError {
    #[error(transparent)]
    Database(#[from] DatabaseError),
    #[error("provider_startup_failed")]
    Provider,
}
