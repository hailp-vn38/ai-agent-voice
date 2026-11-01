//! Admin HTTP handlers for Memory, on top of `MemoryCore`: they parse, call it and map its errors.
//! Nothing here reaches the Wiki, and an id that is not `fact_...` cannot reach a Memory operation.

use crate::{
    app::{
        AppState,
        admin::{
            IntoResponse, Json, Path, Query, Request, Response, State, StatusCode, error, json,
        },
    },
    database::{
        Database,
        devices::{MemoryBinding, memory_binding},
    },
    services::memory::{
        FactId, FactKind, InspectFilter, MemoryActor, MemoryChange, MemoryCore, MemoryError,
        MemoryReceipt, MemoryScope, RecallQuery,
    },
};
use serde::Deserialize;
use std::sync::Arc;

const RECALL_DEFAULT_LIMIT: usize = 20;
const RECALL_MAX_LIMIT: usize = 100;

fn memory_error(req: &Request, cause: MemoryError) -> Response {
    error(req, cause.http_status(), cause.code())
}

fn not_found(req: &Request) -> Response {
    memory_error(req, MemoryError::NotFound)
}

#[allow(clippy::result_large_err)]
fn core(state: &AppState, req: &Request) -> Result<Arc<MemoryCore>, Response> {
    state.memory_core.clone().ok_or_else(|| {
        let cause = if state.config.memory.enabled {
            MemoryError::Unavailable
        } else {
            MemoryError::Disabled
        };
        memory_error(req, cause)
    })
}

async fn binding_of(
    database: &Database,
    device_id: &str,
) -> Result<Option<MemoryBinding>, sqlx::Error> {
    match database.device_row_id(device_id).await? {
        Some(row_id) => memory_binding(database, row_id).await,
        None => Ok(None),
    }
}

fn scope_of(binding: &MemoryBinding) -> MemoryScope {
    MemoryScope::for_stored_device(&binding.device_id, &binding.kind)
}

#[derive(Deserialize)]
pub(in crate::app::admin) struct DeviceQuery {
    device: Option<String>,
}

/// What every Device-scoped route needs, once its `device` parameter is known to be good.
struct DeviceMemory {
    core: Arc<MemoryCore>,
    scope: MemoryScope,
    binding: MemoryBinding,
}

#[allow(clippy::result_large_err)]
async fn device_memory(
    state: &AppState,
    req: Request,
    device: Option<&str>,
) -> Result<(Request, DeviceMemory), Response> {
    let Some(device) = device.filter(|device| !device.is_empty()) else {
        return Err(error(&req, StatusCode::BAD_REQUEST, "device_required"));
    };
    let core = core(state, &req)?;
    let database = crate::app::admin::database(state)?;
    match binding_of(database, device).await {
        Ok(Some(binding)) => Ok((
            req,
            DeviceMemory {
                core,
                scope: scope_of(&binding),
                binding,
            },
        )),
        Ok(None) => Err(error(&req, StatusCode::NOT_FOUND, "device_not_found")),
        Err(cause) => Err(crate::app::admin::sql_error(&req, &cause)),
    }
}

fn receipt_response(req: &Request, result: Result<MemoryReceipt, MemoryError>) -> Response {
    match result {
        Ok(receipt) => Json(receipt).into_response(),
        Err(cause) => memory_error(req, cause),
    }
}

#[derive(Deserialize)]
pub(in crate::app::admin) struct FactsQuery {
    device: Option<String>,
    q: Option<String>,
    limit: Option<usize>,
}

pub(in crate::app::admin) async fn memory_recall(
    State(state): State<AppState>,
    Query(query): Query<FactsQuery>,
    req: Request,
) -> Response {
    let (req, memory) = match device_memory(&state, req, query.device.as_deref()).await {
        Ok(parsed) => parsed,
        Err(response) => return response,
    };
    let text = query.q.as_deref().map(str::trim).unwrap_or_default();
    if text.is_empty() {
        return match memory
            .core
            .inspect(&memory.scope, InspectFilter::default())
            .await
        {
            Ok(overview) => Json(serde_json::json!({ "items": overview.facts })).into_response(),
            Err(cause) => memory_error(&req, cause),
        };
    }
    let recall = RecallQuery {
        text: text.to_string(),
        limit: query
            .limit
            .unwrap_or(RECALL_DEFAULT_LIMIT)
            .clamp(1, RECALL_MAX_LIMIT),
    };
    match memory.core.recall(&memory.scope, recall).await {
        Ok(result) => Json(result).into_response(),
        Err(cause) => memory_error(&req, cause),
    }
}

#[derive(Deserialize)]
struct CreatePayload {
    title: String,
    #[serde(default)]
    hook: String,
    body: String,
    kind: String,
}

pub(in crate::app::admin) async fn memory_create(
    State(state): State<AppState>,
    Query(query): Query<DeviceQuery>,
    req: Request,
) -> Response {
    let (req, input): (_, CreatePayload) = match json(req).await {
        Ok(parsed) => parsed,
        Err(response) => return response,
    };
    let (req, memory) = match device_memory(&state, req, query.device.as_deref()).await {
        Ok(parsed) => parsed,
        Err(response) => return response,
    };
    let Some(kind) = FactKind::parse(&input.kind) else {
        return error(&req, StatusCode::BAD_REQUEST, "invalid_arguments");
    };
    if input.title.trim().is_empty() || input.body.trim().is_empty() {
        return error(&req, StatusCode::BAD_REQUEST, "invalid_arguments");
    }
    let change = MemoryChange::Remember {
        title: input.title,
        hook: input.hook,
        body: input.body,
        kind,
        source_ref: None,
    };
    match memory
        .core
        .apply(&memory.scope, MemoryActor::Admin, change)
        .await
    {
        Ok(receipt) => (StatusCode::CREATED, Json(receipt)).into_response(),
        Err(cause) => memory_error(&req, cause),
    }
}

pub(in crate::app::admin) async fn memory_read(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<DeviceQuery>,
    req: Request,
) -> Response {
    let (req, memory) = match device_memory(&state, req, query.device.as_deref()).await {
        Ok(parsed) => parsed,
        Err(response) => return response,
    };
    let Some(id) = FactId::parse(&id) else {
        return not_found(&req);
    };
    let filter = InspectFilter { id: Some(id) };
    match memory.core.inspect(&memory.scope, filter).await {
        Ok(overview) => match overview.facts.into_iter().next() {
            Some(fact) => Json(fact).into_response(),
            None => not_found(&req),
        },
        Err(cause) => memory_error(&req, cause),
    }
}

/// Applies one change to one fact, after the checks shared by every route under `/facts/{id}`.
async fn apply_to_fact(
    state: &AppState,
    query: &DeviceQuery,
    id: &str,
    req: Request,
    change: impl FnOnce(FactId) -> MemoryChange,
) -> Response {
    let (req, memory) = match device_memory(state, req, query.device.as_deref()).await {
        Ok(parsed) => parsed,
        Err(response) => return response,
    };
    let Some(id) = FactId::parse(id) else {
        return not_found(&req);
    };
    let result = memory
        .core
        .apply(&memory.scope, MemoryActor::Admin, change(id))
        .await;
    receipt_response(&req, result)
}

#[derive(Deserialize)]
struct CorrectPayload {
    title: Option<String>,
    hook: Option<String>,
    body: String,
    expected_revision: u64,
}

pub(in crate::app::admin) async fn memory_update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<DeviceQuery>,
    req: Request,
) -> Response {
    let (req, input): (_, CorrectPayload) = match json(req).await {
        Ok(parsed) => parsed,
        Err(response) => return response,
    };
    let blank_title = input.title.as_deref().is_some_and(|t| t.trim().is_empty());
    if input.body.trim().is_empty() || blank_title {
        return error(&req, StatusCode::BAD_REQUEST, "invalid_arguments");
    }
    apply_to_fact(&state, &query, &id, req, |id| MemoryChange::Correct {
        id,
        expected_revision: input.expected_revision,
        title: input.title,
        hook: input.hook,
        body: input.body,
    })
    .await
}

#[derive(Deserialize)]
struct RevisionPayload {
    expected_revision: u64,
}

pub(in crate::app::admin) async fn memory_forget(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<DeviceQuery>,
    req: Request,
) -> Response {
    let (req, input): (_, RevisionPayload) = match json(req).await {
        Ok(parsed) => parsed,
        Err(response) => return response,
    };
    apply_to_fact(&state, &query, &id, req, |id| MemoryChange::Forget {
        id,
        expected_revision: input.expected_revision,
    })
    .await
}

pub(in crate::app::admin) async fn memory_discard(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<DeviceQuery>,
    req: Request,
) -> Response {
    let (req, input): (_, RevisionPayload) = match json(req).await {
        Ok(parsed) => parsed,
        Err(response) => return response,
    };
    apply_to_fact(&state, &query, &id, req, |id| MemoryChange::Discard {
        id,
        expected_revision: input.expected_revision,
    })
    .await
}

#[derive(Deserialize)]
struct PinPayload {
    pinned: bool,
}

pub(in crate::app::admin) async fn memory_pin(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<DeviceQuery>,
    req: Request,
) -> Response {
    let (req, input): (_, PinPayload) = match json(req).await {
        Ok(parsed) => parsed,
        Err(response) => return response,
    };
    apply_to_fact(&state, &query, &id, req, |id| MemoryChange::SetPinned {
        id,
        pinned: input.pinned,
    })
    .await
}

/// What the Device's Voice Sessions are given, which is what the model actually sees.
pub(in crate::app::admin) async fn memory_summary(
    State(state): State<AppState>,
    Query(query): Query<DeviceQuery>,
    req: Request,
) -> Response {
    let (_, memory) = match device_memory(&state, req, query.device.as_deref()).await {
        Ok(parsed) => parsed,
        Err(response) => return response,
    };
    let context = memory.core.context(&memory.scope).await.current();
    Json(serde_json::json!({
        "summary": context.rows.join("\n"),
        "hidden": context.hidden,
    }))
    .into_response()
}

pub(in crate::app::admin) async fn memory_health(
    State(state): State<AppState>,
    Query(query): Query<DeviceQuery>,
    req: Request,
) -> Response {
    let (req, memory) = match device_memory(&state, req, query.device.as_deref()).await {
        Ok(parsed) => parsed,
        Err(response) => return response,
    };
    match memory
        .core
        .inspect(&memory.scope, InspectFilter::default())
        .await
    {
        Ok(overview) => Json(serde_json::json!({
            "facts": overview.facts.len(),
            "quarantined": overview.quarantined,
            "quarantined_files": overview.quarantined_files,
            "memory_mode": memory.binding.mode,
        }))
        .into_response(),
        Err(cause) => memory_error(&req, cause),
    }
}

pub(in crate::app::admin) async fn memory_rebuild(
    State(state): State<AppState>,
    Query(query): Query<DeviceQuery>,
    req: Request,
) -> Response {
    let (req, memory) = match device_memory(&state, req, query.device.as_deref()).await {
        Ok(parsed) => parsed,
        Err(response) => return response,
    };
    match memory.core.reconcile(&memory.scope).await {
        Ok(report) => Json(report).into_response(),
        Err(cause) => memory_error(&req, cause),
    }
}

pub(in crate::app::admin) async fn memory_profile(
    State(state): State<AppState>,
    req: Request,
) -> Response {
    let core = match core(&state, &req) {
        Ok(core) => core,
        Err(response) => return response,
    };
    match core
        .inspect(&MemoryScope::deployment(), InspectFilter::default())
        .await
    {
        Ok(overview) => Json(overview.profile).into_response(),
        Err(cause) => memory_error(&req, cause),
    }
}

#[derive(Deserialize)]
struct ProfilePayload {
    body: String,
    expected_revision: u64,
}

pub(in crate::app::admin) async fn memory_profile_save(
    State(state): State<AppState>,
    req: Request,
) -> Response {
    let (req, input): (_, ProfilePayload) = match json(req).await {
        Ok(parsed) => parsed,
        Err(response) => return response,
    };
    let core = match core(&state, &req) {
        Ok(core) => core,
        Err(response) => return response,
    };
    let change = MemoryChange::SetProfile {
        body: input.body,
        expected_revision: input.expected_revision,
    };
    let result = core
        .apply(&MemoryScope::deployment(), MemoryActor::Admin, change)
        .await;
    receipt_response(&req, result)
}

/// The vault of a Device about to be deleted. Read before the `DELETE`, because the row is what
/// names the vault; applied only after it succeeded, so a refused delete keeps the vault.
pub(in crate::app::admin) struct DeviceVaultPurge {
    core: Arc<MemoryCore>,
    scope: MemoryScope,
}

impl DeviceVaultPurge {
    pub(in crate::app::admin) async fn prepare(state: &AppState, device_id: &str) -> Option<Self> {
        let core = state.memory_core.clone()?;
        let database = state.database.as_deref()?;
        match binding_of(database, device_id).await {
            Ok(binding) => binding.map(|binding| Self {
                core,
                scope: scope_of(&binding),
            }),
            Err(_) => {
                tracing::warn!(
                    event = "memory_purge_skipped",
                    reason = "binding_unreadable"
                );
                None
            }
        }
    }

    /// The Device is already gone, so a failure cannot undo the request. What is left is reported
    /// as an orphan directory by the next `reconcile_all`.
    pub(in crate::app::admin) async fn run(self) {
        let result = self
            .core
            .apply(&self.scope, MemoryActor::Admin, MemoryChange::PurgeScope)
            .await;
        match result {
            Ok(receipt) if receipt.cleanup_pending => {
                tracing::warn!(event = "memory_purge_incomplete");
            }
            Ok(_) => {}
            Err(cause) => {
                tracing::warn!(event = "memory_purge_failed", code = cause.code());
            }
        }
    }
}
