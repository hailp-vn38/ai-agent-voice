use crate::{
    audio::VadSegmenterConfig,
    config::AppConfig,
    database::secrets::EnvSecretResolver,
    database::{Database, DatabaseError, DesiredProvider, ProviderLoadPlan},
    protocol::{ClientMessage, ServerHello, parse_client_message},
    providers::{
        DatabaseMaterialization, LoadedProviders, ProviderSet, RequiredProviderUnavailable,
        materialize_database_providers,
    },
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

pub use state::{AppState, SessionProfileAdmissionError};

/// Application seam for tests and other callers that have already initialized providers.
pub fn router_with_providers(config: AppConfig, providers: Arc<ProviderSet>) -> Router {
    router_with_state(AppState::from_provider_set(config, providers))
}

/// Public startup seam for deterministic bootstrap tests. SQLite opens, migrates and applies the
/// Provider Load Plan before the application router exists; the injected providers keep model
/// loading outside this seam.
pub async fn bootstrap_with_providers(
    config: AppConfig,
    providers: Arc<ProviderSet>,
) -> Result<Router, BootstrapError> {
    bootstrap_with_providers_and_secret_resolver(config, providers, Arc::new(EnvSecretResolver))
        .await
}

/// Same bootstrap for deployments that own secret resolution, so database provider credentials
/// resolve through the same abstraction production uses.
pub async fn bootstrap_with_providers_and_secret_resolver(
    config: AppConfig,
    providers: Arc<ProviderSet>,
    secret_resolver: Arc<dyn crate::database::secrets::SecretResolver>,
) -> Result<Router, BootstrapError> {
    let database = Database::connect_if_enabled(&config.database).await?;
    let (plan, rows) = read_load_plan(&config, database.as_ref()).await?;
    let (config, loaded) = state::loaded_from_provider_set(config, &providers);
    let (loaded, materialization) =
        apply_load_plan(&config, loaded, rows, &plan, secret_resolver.as_ref())
            .map_err(map_load_plan_failure)?;
    Ok(router_with_state(
        AppState::new_with_database_runtime_snapshot_resolver_and_shutdown(
            config,
            loaded,
            database,
            Some(materialization.snapshot()),
            secret_resolver,
            CancellationToken::new(),
        ),
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
    // The plan and the desired rows are read before any runtime exists, so a required provider is
    // known to be required before the first model is prepared.
    let (plan, rows) = read_load_plan(&config, database.as_ref()).await?;
    let startup_config = config.clone();
    let startup_secret_resolver = Arc::clone(&secret_resolver);
    let (loaded, materialization) = tokio::task::spawn_blocking(move || {
        let local =
            crate::providers::load_local(&startup_config).map_err(|_| BootstrapError::Provider)?;
        apply_load_plan(
            &startup_config,
            local,
            rows,
            &plan,
            startup_secret_resolver.as_ref(),
        )
        .map_err(map_load_plan_failure)
    })
    .await
    .map_err(|_| BootstrapError::Provider)??;
    Ok(router_with_state(
        AppState::new_with_database_runtime_snapshot_resolver_and_shutdown(
            config,
            loaded,
            database,
            Some(materialization.snapshot()),
            secret_resolver,
            shutdown,
        ),
    ))
}

/// Derives the Provider Load Plan from the persisted graph plus the deployment's server provider
/// defaults.  Requirements never come from `AppConfig::effective_agent`, whose overrides describe
/// a single process-local agent rather than database intent.
async fn read_load_plan(
    config: &AppConfig,
    database: Option<&Database>,
) -> Result<(ProviderLoadPlan, Vec<DesiredProvider>), BootstrapError> {
    let Some(database) = database else {
        return Ok((
            ProviderLoadPlan::from_server_defaults(&config.provider_defaults),
            Vec::new(),
        ));
    };
    let plan = database
        .provider_load_plan(&config.provider_defaults)
        .await?;
    let rows = database.enabled_provider_rows().await?;
    Ok((plan, rows))
}

/// Folds the plan's database runtimes into the already-loaded snapshot.
///
/// A required provider whose key the deployment already occupies can never be resolved
/// unambiguously, so it is rejected before any credential is resolved or any model is prepared.
/// An optional collision is just an optional failure: the database instance stays unavailable and
/// its dependent non-default candidate is excluded, while boot continues.
fn apply_load_plan(
    config: &AppConfig,
    mut loaded: LoadedProviders,
    rows: Vec<DesiredProvider>,
    plan: &ProviderLoadPlan,
    secrets: &dyn crate::database::secrets::SecretResolver,
) -> Result<(LoadedProviders, DatabaseMaterialization), RequiredProviderUnavailable> {
    if let Some(row) = rows
        .iter()
        .find(|row| plan.is_required(&row.key) && loaded.has_loaded_key(&row.kind, &row.key))
    {
        return Err(RequiredProviderUnavailable {
            provider_key: row.key.clone(),
            failure: crate::providers::DatabaseRuntimeFailure::Runtime,
        });
    }
    let mut materialization = materialize_database_providers(config, rows, plan, secrets)?;
    for collision in loaded.extend_database_without_collisions(materialization.take_loaded()) {
        materialization.mark_unavailable(
            &collision,
            crate::providers::DatabaseRuntimeFailure::Runtime,
        );
    }
    Ok((loaded, materialization))
}

fn map_load_plan_failure(error: RequiredProviderUnavailable) -> BootstrapError {
    tracing::error!(
        provider_key = %error.provider_key,
        reason = ?error.failure,
        "required database provider is unavailable; refusing to bind the listener"
    );
    BootstrapError::Provider
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
