use super::*;
use crate::app::assistant::trusted_origin;
use crate::database::devices::Device;

fn settings(device: Option<Device>, active: usize) -> Value {
    match device {
        Some(d) => {
            serde_json::json!({"configured":true,"enabled":d.enabled==1,"device_id":d.device_id,"kind":d.kind,"agent_key":d.agent_key,"template_key":d.template_key,"revision":d.revision,"active_connections":active})
        }
        None => {
            serde_json::json!({"configured":false,"enabled":false,"device_id":null,"agent_key":null,"template_key":null,"revision":null,"active_connections":active})
        }
    }
}
pub(super) async fn get(State(state): State<AppState>, request: Request) -> Response {
    let db = match database(&state) {
        Ok(db) => db,
        Err(_) => {
            return error(
                &request,
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
            );
        }
    };
    match db.assistant_device().await {
        Ok(device) => Json(settings(device, state.web_assistant.active())).into_response(),
        Err(e) => sql_error(&request, &e),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    enabled: bool,
    agent_key: String,
    #[serde(default)]
    template_key: Option<String>,
}
pub(super) async fn put(State(state): State<AppState>, request: Request) -> Response {
    let (request, input): (_, Input) = match json(request).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    let expected = if request.headers().contains_key(header::IF_MATCH) {
        match expected(request.headers()) {
            Ok(v) => Some(v),
            Err(c) => return error(&request, StatusCode::BAD_REQUEST, c),
        }
    } else {
        None
    };
    let db = match database(&state) {
        Ok(db) => db,
        Err(_) => {
            return error(
                &request,
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
            );
        }
    };
    let _publication = state.web_assistant.publication.write().await;
    match db
        .save_assistant_device(
            &input.agent_key,
            input.template_key.as_deref(),
            input.enabled,
            expected,
            id(&request),
        )
        .await
    {
        Ok(device) => {
            state.web_assistant.invalidate();
            Json(settings(Some(device), state.web_assistant.active())).into_response()
        }
        Err(cause) => write_error(&request, cause),
    }
}
pub(super) async fn ticket(State(state): State<AppState>, request: Request) -> Response {
    if !state.admission_gate().is_open() {
        return error(&request, StatusCode::SERVICE_UNAVAILABLE, "shutting_down");
    }
    let Some(origin) = trusted_origin(&state, request.headers()) else {
        return error(&request, StatusCode::FORBIDDEN, "untrusted_origin");
    };
    let _publication = state.web_assistant.publication.read().await;
    let db = match database(&state) {
        Ok(db) => db,
        Err(_) => {
            return error(
                &request,
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
            );
        }
    };
    let device = match db.assistant_device().await {
        Ok(Some(d)) if d.enabled == 1 => d,
        Ok(Some(_)) => return error(&request, StatusCode::CONFLICT, "assistant_disabled"),
        Ok(None) => return error(&request, StatusCode::CONFLICT, "assistant_not_configured"),
        Err(e) => return sql_error(&request, &e),
    };
    if !state.admission_gate().is_open() {
        return error(&request, StatusCode::SERVICE_UNAVAILABLE, "shutting_down");
    }
    match state.web_assistant.issue(&device, &origin) {
        Ok(ticket) => ([(header::CACHE_CONTROL, "no-store")], Json(ticket)).into_response(),
        Err(code) => error(
            &request,
            if code == "assistant_rate_limited" {
                StatusCode::TOO_MANY_REQUESTS
            } else {
                StatusCode::SERVICE_UNAVAILABLE
            },
            code,
        ),
    }
}
