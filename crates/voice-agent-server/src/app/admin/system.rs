//! Safe aggregate Admin status; it deliberately excludes paths, configs, secrets and failures.
use super::*;

pub(super) async fn get_system(State(state): State<AppState>, request: Request) -> Response {
    if state.database.is_none() {
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
        );
    }
    match state.system_status().summary().await {
        Ok(summary) => Json(summary).into_response(),
        Err(cause) => sql_error(&request, &cause),
    }
}

pub(super) async fn get_resources(State(state): State<AppState>, request: Request) -> Response {
    let Some(snapshot) = state.system_metrics.latest() else {
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "system_metrics_unavailable",
        );
    };
    let mut response = Json(snapshot).into_response();
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response
}
