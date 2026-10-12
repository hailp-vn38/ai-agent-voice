use super::*;
use crate::database::notifications::{CreateReminder, DeviceNotifications, NotificationError};
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
    match DeviceNotifications::new(database.clone())
        .create(&device, "admin", key, input, "web", id(&request))
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
    match DeviceNotifications::new(database.clone())
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
    match DeviceNotifications::new(db.clone())
        .update(
            &device,
            &reminder,
            revision,
            "admin",
            key,
            input,
            id(&request),
        )
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
    match DeviceNotifications::new(db.clone())
        .cancel(&device, &reminder, revision, "admin", key, id(&request))
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
