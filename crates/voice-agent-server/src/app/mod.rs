use crate::{
    audio::VadSegmenterConfig,
    config::AppConfig,
    database::secrets::EnvSecretResolver,
    database::{Database, DatabaseError, DesiredProvider, ProviderLoadPlan},
    lifecycle::RuntimeLifecycle,
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
mod cors;
mod enrollment;
mod ota;
mod state;
mod vision;
mod websocket;

pub use state::{AppState, SessionProfileAdmissionError};

/// Application seam for tests and other callers that have already initialized providers.
pub async fn router_with_providers(
    config: AppConfig,
    providers: Arc<ProviderSet>,
) -> Result<Router, BootstrapError> {
    bootstrap_with_providers(config, providers).await
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
    let database = Database::connect(&config.database).await?;
    let (plan, rows) = read_load_plan(&config, &database).await?;
    let (config, loaded) = state::loaded_from_provider_set(config, &providers);
    let (loaded, materialization) =
        apply_load_plan(&config, loaded, rows, &plan, secret_resolver.as_ref())
            .map_err(map_load_plan_failure)?;
    let lifecycle = new_lifecycle(&config);
    let mut state = AppState::new_with_database_runtime_snapshot_resolver_and_shutdown(
        config,
        loaded,
        Some(database),
        Some(materialization.snapshot()),
        secret_resolver,
        lifecycle,
    );
    prepare_enrollment(&mut state).await?;
    Ok(router_with_state(state))
}

/// A lifecycle that owns its own signals, for a process that has nothing outside it to stop.
pub fn new_lifecycle(config: &AppConfig) -> Arc<RuntimeLifecycle> {
    RuntimeLifecycle::new(Duration::from_millis(config.shutdown.grace_ms))
}

/// The same, from the raw configured grace.  Test routers that build their own `AppConfig` use
/// this so they run under the deadline their configuration actually names.
pub fn new_lifecycle_from_grace_ms(grace_ms: u64) -> Arc<RuntimeLifecycle> {
    RuntimeLifecycle::new(Duration::from_millis(grace_ms))
}
pub fn router_with_state(state: AppState) -> Router {
    let vision_enabled = state.config.vision.enabled;
    let vision_config = state.config.vision.clone();
    let router = Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route_layer(cors::layer())
        .route(
            "/voice/ota/",
            get(ota::handler)
                .post(ota::handler)
                .options(ota::options)
                .layer(middleware::from_fn(ota::cors_response)),
        )
        .route("/voice/v1/", get(websocket::handler));
    let router = if state.config.database.devices.enrollment.enabled {
        router
            .route(
                "/voice/ota",
                get(ota::handler)
                    .post(ota::handler)
                    .options(ota::options)
                    .layer(middleware::from_fn(ota::cors_response)),
            )
            .route(
                "/voice/ota/activate",
                axum::routing::post(ota::activate)
                    .options(ota::options)
                    .layer(middleware::from_fn(ota::cors_response)),
            )
            .route(
                "/voice/ota/activate/",
                axum::routing::post(ota::activate)
                    .options(ota::options)
                    .layer(middleware::from_fn(ota::cors_response)),
            )
    } else {
        router
    };
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
    let router = if state.config.api.enabled {
        router.nest(
            "/api/admin",
            admin::router(state.clone()).layer(cors::layer()),
        )
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
pub async fn application(config: AppConfig) -> Result<Router, BootstrapError> {
    startup(config).await
}

/// Production startup orders the database dependency before provider initialization and listener
/// binding. Database failure is therefore always a pre-bind failure.
pub async fn startup(config: AppConfig) -> Result<Router, BootstrapError> {
    let lifecycle = RuntimeLifecycle::from_config(&config);
    startup_with_lifecycle(config, lifecycle).await
}

/// Production startup on a lifecycle the caller owns.
///
/// Handing the lifecycle in rather than building it here is what lets the process owner drive the
/// same ordered shutdown the router's own components see: it holds the gate the WebSocket boundary,
/// the admission resolver and the External MCP limiter all ask, and the drain registry those
/// sessions register with. A caller that only wants a router uses [`startup`].
pub async fn startup_with_lifecycle(
    config: AppConfig,
    lifecycle: Arc<RuntimeLifecycle>,
) -> Result<Router, BootstrapError> {
    startup_with_lifecycle_and_secret_resolver(config, lifecycle, Arc::new(EnvSecretResolver)).await
}

/// Production startup seam for deployments that own secret resolution.  The runtime receives
/// only this abstraction; provider adapters never read a resolver backend directly.
pub async fn startup_with_lifecycle_and_secret_resolver(
    config: AppConfig,
    lifecycle: Arc<RuntimeLifecycle>,
    secret_resolver: Arc<dyn crate::database::secrets::SecretResolver>,
) -> Result<Router, BootstrapError> {
    let database = Database::connect(&config.database).await?;
    let (plan, rows) = read_load_plan(&config, &database).await?;
    config.validate().map_err(|_| BootstrapError::Provider)?;
    // Every local provider this deployment declares gets its model files on disk before the server
    // binds, whether or not its runtime is built yet. Files already present are reused, so this
    // costs nothing after the first start and never hashes a model.
    let declared = config.clone();
    tokio::task::spawn_blocking(move || ensure_declared_assets(&declared))
        .await
        .map_err(|_| BootstrapError::Provider)?
        .map_err(|_| BootstrapError::Provider)?;
    if config.provider_runtime.is_some() {
        return managed_startup(config, database, secret_resolver, lifecycle).await;
    }
    // Provider runtimes are built here; their model files are already on disk from the pass above.
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
    let mut state = AppState::new_with_database_runtime_snapshot_resolver_and_shutdown(
        config,
        loaded,
        Some(database),
        Some(materialization.snapshot()),
        secret_resolver,
        lifecycle,
    );
    prepare_enrollment(&mut state).await?;
    Ok(router_with_state(state))
}

/// Installs the model files of every local provider the deployment declares.
///
/// This is the whole of startup's model handling: each provider knows its own files, and a file
/// that exists with content is reused without a request. Nothing is scanned, hashed or checksummed,
/// and a provider that publishes no files — every remote adapter — is skipped.
///
/// A declared provider whose files cannot be installed is a startup failure. The operator asked
/// for that model by configuring it, so finding out at boot beats discovering it on a request.
fn ensure_declared_assets(config: &AppConfig) -> Result<(), crate::providers::ProviderLoadError> {
    let registry = crate::providers::compiled_provider_adapter_registry();
    let mut ensured: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for adapter in configured_local_adapters(config) {
        // Several instances may share one adapter; one ensure covers them all.
        if !ensured.insert(adapter) {
            continue;
        }
        let Some(assets) = registry.assets(adapter) else {
            continue;
        };
        assets.ensure_assets()?;
        tracing::info!(provider = adapter, "provider model files ready");
    }
    Ok(())
}

/// Adapters of every provider instance the deployment configures, deduplicated by adapter.
fn configured_local_adapters(config: &AppConfig) -> Vec<&'static str> {
    let mut adapters: Vec<&'static str> = config
        .providers
        .vad
        .instances
        .values()
        .map(|instance| instance.adapter())
        .chain(
            config
                .providers
                .asr
                .instances
                .values()
                .map(|instance| instance.adapter()),
        )
        .chain(
            config
                .providers
                .tts
                .instances
                .values()
                .map(|instance| instance.adapter()),
        )
        .collect();
    adapters.sort_unstable();
    adapters.dedup();
    adapters
}

/// Providers that must be ready before the server binds.
///
/// The deployment defaults for every kind, plus any TTS instance that asked to be preloaded. Each
/// one is acquired here so its model files are installed and its runtime is resident: a deployment
/// starts into a known-good state rather than making the first request pay for a download. A
/// provider that fails to materialize is a startup failure, not a deferred surprise.
pub fn startup_providers(config: &AppConfig) -> std::collections::BTreeSet<(String, String)> {
    let defaults = &config.provider_defaults;
    let mut required: std::collections::BTreeSet<(String, String)> = [
        ("vad", &defaults.vad),
        ("asr", &defaults.asr),
        ("llm", &defaults.llm),
        ("tts", &defaults.tts),
    ]
    .into_iter()
    .map(|(kind, key)| (kind.to_owned(), key.clone()))
    .collect();
    for (key, instance) in &config.providers.tts.instances {
        if instance.preload() {
            required.insert(("tts".to_owned(), key.clone()));
        }
    }
    required
}

async fn managed_startup(
    config: AppConfig,
    database: Database,
    secret_resolver: Arc<dyn crate::database::secrets::SecretResolver>,
    lifecycle: Arc<RuntimeLifecycle>,
) -> Result<Router, BootstrapError> {
    use crate::services::provider_runtime::{FactoryMaterializer, ProviderRuntimeManager};
    let runtime_config = config
        .provider_runtime
        .as_ref()
        .ok_or(BootstrapError::Provider)?;
    // No catalog or provider set pins graphs beside the manager on this path.
    let loaded = LoadedProviders {
        providers: Default::default(),
        runtimes: Default::default(),
    };
    let mut state = AppState::new_with_database_runtime_snapshot_resolver_and_shutdown(
        config.clone(),
        loaded,
        Some(database),
        None,
        secret_resolver.clone(),
        lifecycle.clone(),
    );
    let builder = FactoryMaterializer::new(
        Arc::new(config.clone()),
        secret_resolver,
        runtime_config.estimated_peak_bytes.clone(),
        state.worker_supervisor.clone(),
    )
    .map_err(|_| BootstrapError::Provider)?;
    let manager = ProviderRuntimeManager::new(
        runtime_config.limits.clone(),
        Arc::new(builder),
        lifecycle.gate().clone(),
    )
    .map_err(|_| BootstrapError::Provider)?;
    state = state.with_runtime_manager(manager.clone());
    let startup_deadline =
        tokio::time::Instant::now() + Duration::from_millis(runtime_config.startup_timeout_ms);
    for (kind, key) in startup_providers(&config) {
        let snapshot = crate::providers::deployment_provider_snapshot(&config, &kind, &key)
            .map_err(|_| BootstrapError::Provider)?;
        let lease = manager
            .acquire_deployment_until(snapshot.clone(), startup_deadline)
            .await
            .map_err(|_| BootstrapError::Provider)?;
        manager
            .retain_deployment(lease.version())
            .map_err(|_| BootstrapError::Provider)?;
        state.deployment_snapshots.push(snapshot);
        drop(lease);
    }
    if let (Some(database), Some(prewarm)) = (&state.database, &state.provider_prewarm) {
        let defaults: Vec<(i64,)> = sqlx::query_as("SELECT DISTINCT t.id FROM agent_templates t JOIN agent_template_assignments a ON a.template_id=t.id JOIN agents g ON g.id=a.agent_id WHERE a.enabled=1 AND a.is_default=1 AND t.enabled=1 AND g.enabled=1 LIMIT 256").fetch_all(database.pool()).await.map_err(|_| BootstrapError::Provider)?;
        for (id,) in defaults {
            prewarm.template(id).await;
        }
    }
    prepare_enrollment(&mut state).await?;
    Ok(router_with_state(state))
}

async fn prepare_enrollment(state: &mut AppState) -> Result<(), BootstrapError> {
    state.enrollment_runtime = crate::services::device_enrollment::EnrollmentRuntime::prepare(
        &state.config.database.devices.enrollment,
    )
    .await
    .map_err(|_| BootstrapError::Enrollment)?;
    Ok(())
}

/// Derives the Provider Load Plan from the persisted graph plus the deployment's server provider
/// defaults.  Requirements never come from `AppConfig::effective_agent`, whose overrides describe
/// a single process-local agent rather than database intent.
async fn read_load_plan(
    config: &AppConfig,
    database: &Database,
) -> Result<(ProviderLoadPlan, Vec<DesiredProvider>), BootstrapError> {
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

/// Liveness.  It answers exactly one question — is this process running — and deliberately
/// depends on nothing else.
///
/// A database that has gone away, an External MCP server that is refusing connections, or a
/// shutdown that has begun must not make this route fail: doing so would tell an orchestrator to
/// kill a process that is still serving the Voice Sessions it already admitted, and those sessions
/// hold the only copy of their Dialogue History.
async fn health() -> &'static str {
    "ok"
}

/// Readiness.  It answers whether this process can accept a *new* connection, and reports only the
/// application-owned dependencies that decide that.
///
/// The route is deliberately thin: it delegates to [`AppState::readiness`] and returns the bounded
/// class it produced.  There is no admission, no Device lookup, no External MCP discovery and no
/// provider or secret reload on this path — see that method for why each of those belongs to a
/// session instead of to a probe.
async fn ready(State(state): State<AppState>) -> Response {
    let readiness = state.readiness().await;
    if !readiness.is_ready() {
        warn!(
            event = "readiness_degraded",
            reason = readiness.as_str(),
            "The process is running but cannot accept a new Voice connection"
        );
        return (StatusCode::SERVICE_UNAVAILABLE, readiness.as_str()).into_response();
    }
    (StatusCode::OK, "ready").into_response()
}

#[derive(Debug, Error)]
pub enum BootstrapError {
    #[error(transparent)]
    Database(#[from] DatabaseError),
    #[error("provider_startup_failed")]
    Provider,
    #[error("enrollment_prompt_startup_failed: prepare 24kHz mono PCM16 WAV assets")]
    Enrollment,
}
