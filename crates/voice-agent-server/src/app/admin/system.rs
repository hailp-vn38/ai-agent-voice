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
        return Json(serde_json::json!({"status":state.readiness().await.as_str(),"version":env!("CARGO_PKG_VERSION"),"uptime_seconds":state.started_at.elapsed().as_secs(),"database":{"enabled":true,"status":"unavailable"},"providers":{"configured":null,"loaded":null,"stale":null,"failed":null},"sessions":{"active":state.lifecycle.drain().active()},"provider_runtime":state.provider_runtime_manager.as_ref().map(|manager|serde_json::json!({"accounting":manager.accounting(),"metrics":manager.metrics().snapshot()}))})).into_response();
    }
    let configured: i64 = match sqlx::query_scalar("SELECT COUNT(*) FROM providers")
        .fetch_one(database.pool())
        .await
    {
        Ok(value) => value,
        Err(value) => return sql_error(&request, &value),
    };
    let rows: Vec<(i64, String, i64)> = if let Some(manager) = &state.provider_runtime_manager {
        let ids = manager.tracked_database_ids();
        if ids.is_empty() {
            Vec::new()
        } else {
            let mut query = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
                "SELECT id, '', revision FROM providers WHERE id IN (",
            );
            let mut values = query.separated(",");
            for id in ids {
                values.push_bind(id);
            }
            values.push_unseparated(")");
            match query.build_query_as().fetch_all(database.pool()).await {
                Ok(value) => value,
                Err(value) => return sql_error(&request, &value),
            }
        }
    } else {
        match sqlx::query_as("SELECT id, key, revision FROM providers")
            .fetch_all(database.pool())
            .await
        {
            Ok(value) => value,
            Err(value) => return sql_error(&request, &value),
        }
    };
    let (mut loaded, mut stale, mut failed) = (0_u64, 0_u64, 0_u64);
    for (provider_id, key, revision) in &rows {
        if let Some(manager) = &state.provider_runtime_manager {
            match manager.inspect(*provider_id, *revision).desired_state {
                crate::services::provider_runtime::RuntimeState::Ready => loaded += 1,
                crate::services::provider_runtime::RuntimeState::Failed
                | crate::services::provider_runtime::RuntimeState::Quarantined => failed += 1,
                _ => stale += 1,
            }
            continue;
        }
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
    if state.provider_runtime_manager.is_some() {
        stale = (configured as u64).saturating_sub(loaded + failed);
    }
    Json(serde_json::json!({"status":state.readiness().await.as_str(),"version":env!("CARGO_PKG_VERSION"),"uptime_seconds":state.started_at.elapsed().as_secs(),"database":{"enabled":true,"status":database_status},"providers":{"configured":configured,"loaded":loaded,"stale":stale,"failed":failed},"sessions":{"active":state.lifecycle.drain().active()},"provider_runtime":state.provider_runtime_manager.as_ref().map(|manager|serde_json::json!({"accounting":manager.accounting(),"metrics":manager.metrics().snapshot()}))})).into_response()
}
