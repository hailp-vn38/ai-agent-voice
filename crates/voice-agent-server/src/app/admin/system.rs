//! Safe aggregate Admin status; it deliberately excludes paths, configs, secrets and failures.
use super::*;

pub(super) async fn get_system(State(state): State<AppState>, request: Request) -> Response {
    let Some(database) = state.database.as_ref() else {
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
        );
    };
    let database_status = if database.is_reachable().await.is_ok() {
        "ready"
    } else {
        "unavailable"
    };
    if database_status == "unavailable" {
        return Json(serde_json::json!({"status":state.readiness().await.as_str(),"version":env!("CARGO_PKG_VERSION"),"uptime_seconds":state.started_at.elapsed().as_secs(),"database":{"enabled":true,"status":"unavailable"},"providers":{"configured":null,"loaded":null,"stale":null,"failed":null},"sessions":{"active":state.lifecycle.drain().active()}})).into_response();
    }
    let rows: Vec<(String, i64)> = match sqlx::query_as("SELECT key, revision FROM providers")
        .fetch_all(database.pool())
        .await
    {
        Ok(value) => value,
        Err(value) => return sql_error(&request, &value),
    };
    let (mut loaded, mut stale, mut failed) = (0_u64, 0_u64, 0_u64);
    for (key, revision) in &rows {
        match state
            .database_runtime_snapshot
            .as_deref()
            .map(|snapshot| snapshot.runtime_state(key, *revision).status)
        {
            Some(crate::providers::DatabaseRuntimeStatus::Loaded) => loaded += 1,
            Some(crate::providers::DatabaseRuntimeStatus::Unavailable) => failed += 1,
            Some(crate::providers::DatabaseRuntimeStatus::NotLoaded) | None => stale += 1,
        }
    }
    Json(serde_json::json!({"status":state.readiness().await.as_str(),"version":env!("CARGO_PKG_VERSION"),"uptime_seconds":state.started_at.elapsed().as_secs(),"database":{"enabled":true,"status":database_status},"providers":{"configured":rows.len(),"loaded":loaded,"stale":stale,"failed":failed},"sessions":{"active":state.lifecycle.drain().active()}})).into_response()
}
