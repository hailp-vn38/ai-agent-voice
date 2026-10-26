use super::*;
use crate::{
    database::notifications::{
        CreateReminder, DeviceNotifications, MutationContext, NotificationError, ReminderSource,
    },
    services::notifications::ReminderCoordinator,
};
fn failure(request: &Request, cause: NotificationError) -> Response {
    let (status, code) = match cause {
        NotificationError::Invalid("unauthorized_target") => {
            (StatusCode::FORBIDDEN, "unauthorized_target")
        }
        NotificationError::Invalid(code) => (StatusCode::BAD_REQUEST, code),
        NotificationError::NotFound => (StatusCode::NOT_FOUND, "not_found"),
        NotificationError::Conflict(code) => (StatusCode::CONFLICT, code),
        NotificationError::Sql(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            "device_notifications_unavailable",
        ),
    };
    error(request, status, code)
}
pub(super) async fn create(
    State(state): State<AppState>,
    Path(device): Path<String>,
    request: Request,
) -> Response {
    let (request, input): (_, CreateReminder) = match json(request).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    let key = match mutation_key(&request) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let database = match database(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let ctx = MutationContext {
        caller: "admin",
        request_id: key,
        audit_request_id: id(&request),
        source: ReminderSource::Web,
        agent_scope: agent_scope(&request),
    };
    match coordinator(&state, database, &request)
        .create(&device, &ctx, input)
        .await
    {
        Ok(v) => (StatusCode::CREATED, Json(v)).into_response(),
        Err(e) => failure(&request, e),
    }
}
#[derive(Deserialize)]
pub(super) struct ListQuery {
    page: Option<u32>,
    page_size: Option<u32>,
    status: Option<String>,
}
pub(super) async fn list(
    State(state): State<AppState>,
    Path(device): Path<String>,
    Query(query): Query<ListQuery>,
    request: Request,
) -> Response {
    let database = match database(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    match coordinator(&state, database, &request)
        .list(
            &device,
            query.status.as_deref(),
            query.page.unwrap_or(1),
            query.page_size.unwrap_or(50),
        )
        .await
    {
        Ok(v) => Json(v).into_response(),
        Err(e) => failure(&request, e),
    }
}
pub(super) async fn grant(
    State(state): State<AppState>,
    Path(key): Path<String>,
    request: Request,
) -> Response {
    let database = match database(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    match DeviceNotifications::new(database.clone()).grant(&key).await {
        Ok((enabled, revision)) => {
            Json(serde_json::json!({"reminders_enabled":enabled,"revision":revision}))
                .into_response()
        }
        Err(e) => failure(&request, e),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GrantPatch {
    reminders_enabled: bool,
}
pub(super) async fn patch_grant(
    State(state): State<AppState>,
    Path(key): Path<String>,
    request: Request,
) -> Response {
    let (request, input): (_, GrantPatch) = match json(request).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    let revision = match expected(request.headers()) {
        Ok(v) => v,
        Err(e) => return error(&request, StatusCode::BAD_REQUEST, e),
    };
    let database = match database(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    match DeviceNotifications::new(database.clone())
        .set_grant(&key, input.reminders_enabled, revision, id(&request))
        .await
    {
        Ok(revision) => Json(
            serde_json::json!({"reminders_enabled":input.reminders_enabled,"revision":revision}),
        )
        .into_response(),
        Err(e) => failure(&request, e),
    }
}

pub(super) async fn update(
    State(state): State<AppState>,
    Path((device, reminder)): Path<(String, String)>,
    request: Request,
) -> Response {
    let (request, input): (_, crate::database::notifications::UpdateReminder) =
        match json(request).await {
            Ok(v) => v,
            Err(e) => return e,
        };
    let revision = match expected(request.headers()) {
        Ok(v) => v,
        Err(e) => return error(&request, StatusCode::BAD_REQUEST, e),
    };
    let key = match mutation_key(&request) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let db = match database(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let ctx = MutationContext {
        caller: "admin",
        request_id: key,
        audit_request_id: id(&request),
        source: ReminderSource::Web,
        agent_scope: agent_scope(&request),
    };
    match coordinator(&state, db, &request)
        .update(&device, &reminder, revision, &ctx, input)
        .await
    {
        Ok(v) => Json(v).into_response(),
        Err(e) => failure(&request, e),
    }
}
pub(super) async fn cancel(
    State(state): State<AppState>,
    Path((device, reminder)): Path<(String, String)>,
    request: Request,
) -> Response {
    let (request, input): (_, Value) = match json(request).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    if !input.as_object().is_some_and(|v| v.is_empty()) {
        return error(&request, StatusCode::BAD_REQUEST, "validation_failed");
    }
    let revision = match expected(request.headers()) {
        Ok(v) => v,
        Err(e) => return error(&request, StatusCode::BAD_REQUEST, e),
    };
    let key = match mutation_key(&request) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let db = match database(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let ctx = MutationContext {
        caller: "admin",
        request_id: key,
        audit_request_id: id(&request),
        source: ReminderSource::Web,
        agent_scope: agent_scope(&request),
    };
    match coordinator(&state, db, &request)
        .cancel(&device, &reminder, revision, &ctx)
        .await
    {
        Ok(v) => Json(v).into_response(),
        Err(e) => failure(&request, e),
    }
}
#[allow(clippy::result_large_err)]
fn mutation_key(request: &Request) -> Result<&str, Response> {
    request
        .headers()
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| error(request, StatusCode::BAD_REQUEST, "idempotency_key_required"))
}

#[derive(Clone)]
struct AgentScope(String);

fn agent_scope(request: &Request) -> Option<&str> {
    request
        .extensions()
        .get::<AgentScope>()
        .map(|s| s.0.as_str())
}

fn coordinator(
    state: &AppState,
    database: &crate::database::Database,
    request: &Request,
) -> ReminderCoordinator {
    let service = ReminderCoordinator::from_database(
        database.clone(),
        std::sync::Arc::clone(&state.active_device_sessions),
    );
    match agent_scope(request) {
        Some(scope) => service.scoped_to_agent(scope),
        None => service,
    }
}

pub(super) async fn agent_list(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Query(query): Query<crate::database::notifications::AgentReminderQuery>,
    request: Request,
) -> Response {
    let db = match database(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    match ReminderCoordinator::from_database(
        db.clone(),
        std::sync::Arc::clone(&state.active_device_sessions),
    )
    .list_agent(&key, &query)
    .await
    {
        Ok(v) => Json(v).into_response(),
        Err(e) => failure(&request, e),
    }
}
pub(super) async fn scoped_create(
    state: State<AppState>,
    Path((agent, device)): Path<(String, String)>,
    mut request: Request,
) -> Response {
    request.extensions_mut().insert(AgentScope(agent));
    create(state, Path(device), request).await
}
pub(super) async fn scoped_list(
    state: State<AppState>,
    Path((agent, device)): Path<(String, String)>,
    query: Query<ListQuery>,
    mut request: Request,
) -> Response {
    request.extensions_mut().insert(AgentScope(agent));
    list(state, Path(device), query, request).await
}
pub(super) async fn scoped_update(
    state: State<AppState>,
    Path((agent, device, reminder)): Path<(String, String, String)>,
    mut request: Request,
) -> Response {
    request.extensions_mut().insert(AgentScope(agent));
    update(state, Path((device, reminder)), request).await
}
pub(super) async fn scoped_cancel(
    state: State<AppState>,
    Path((agent, device, reminder)): Path<(String, String, String)>,
    mut request: Request,
) -> Response {
    request.extensions_mut().insert(AgentScope(agent));
    cancel(state, Path((device, reminder)), request).await
}
