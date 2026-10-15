use super::*;
use crate::services::background::{BackgroundSettingsUpdate, JobError, JobRequest, JobRequester};
use serde::Deserialize;

fn runtime(state: &AppState) -> Result<&crate::services::background::JobEngine, JobError> {
    state
        .background
        .as_ref()
        .map(|r| r.engine.as_ref())
        .ok_or(JobError::RuntimeUnavailable)
}
fn failure(request: &Request, cause: JobError) -> Response {
    let status = match cause {
        JobError::InvalidArguments => StatusCode::BAD_REQUEST,
        JobError::AuthorityRevoked => StatusCode::FORBIDDEN,
        JobError::NotFound => StatusCode::NOT_FOUND,
        JobError::RevisionConflict
        | JobError::IdempotencyConflict
        | JobError::Terminal
        | JobError::DeleteConflict => StatusCode::CONFLICT,
        JobError::QueueFull => StatusCode::TOO_MANY_REQUESTS,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    };
    error(request, status, cause.code())
}
pub(super) async fn settings(State(state): State<AppState>, request: Request) -> Response {
    match runtime(&state) {
        Ok(engine) => match engine.settings().await {
            Ok(s) => Json(s).into_response(),
            Err(e) => failure(&request, e),
        },
        Err(e) => failure(&request, e),
    }
}
pub(super) async fn save_settings(State(state): State<AppState>, request: Request) -> Response {
    let expected = match expected(request.headers()) {
        Ok(v) => v,
        Err(e) => return error(&request, StatusCode::BAD_REQUEST, e),
    };
    let (request, input): (_, BackgroundSettingsUpdate) = match json(request).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    match runtime(&state) {
        Ok(engine) => match engine.update_settings(input, expected).await {
            Ok(s) => Json(s).into_response(),
            Err(e) => failure(&request, e),
        },
        Err(e) => failure(&request, e),
    }
}
pub(super) async fn create(State(state): State<AppState>, request: Request) -> Response {
    let key = match request.headers().get("Idempotency-Key") {
        Some(v) => match v.to_str() {
            Ok(v) => Some(v.to_owned()),
            Err(_) => return error(&request, StatusCode::BAD_REQUEST, "invalid_arguments"),
        },
        None => None,
    };
    let (request, input): (_, JobRequest) = match json(request).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    match runtime(&state) {
        Ok(engine) => match engine
            .submit(input, &JobRequester::Admin, key.as_deref())
            .await
        {
            Ok(receipt) => (
                if receipt.replayed {
                    StatusCode::OK
                } else {
                    StatusCode::CREATED
                },
                Json(receipt.job),
            )
                .into_response(),
            Err(e) => failure(&request, e),
        },
        Err(e) => failure(&request, e),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct JobQuery {
    page: Option<u32>,
    page_size: Option<u32>,
    status: Option<String>,
    q: Option<String>,
}
pub(super) async fn list(
    State(state): State<AppState>,
    Query(query): Query<JobQuery>,
    request: Request,
) -> Response {
    let page = query.page.unwrap_or(1);
    let size = query.page_size.unwrap_or(30);
    match runtime(&state) {
        Ok(engine) => match engine
            .list(page, size, query.status.as_deref(), query.q.as_deref())
            .await
        {
            Ok((items, total)) => {
                Json(serde_json::json!({"items":items,"total":total,"page":page,"page_size":size}))
                    .into_response()
            }
            Err(e) => failure(&request, e),
        },
        Err(e) => failure(&request, e),
    }
}
pub(super) async fn get(
    State(state): State<AppState>,
    Path(id): Path<String>,
    request: Request,
) -> Response {
    match runtime(&state) {
        Ok(engine) => match engine.get(&id).await {
            Ok(job) => Json(job).into_response(),
            Err(e) => failure(&request, e),
        },
        Err(e) => failure(&request, e),
    }
}
pub(super) async fn delete(
    State(state): State<AppState>,
    Path(id): Path<String>,
    request: Request,
) -> Response {
    match runtime(&state) {
        Ok(engine) => match engine.delete(&id).await {
            Ok(()) => StatusCode::NO_CONTENT.into_response(),
            Err(cause) => failure(&request, cause),
        },
        Err(cause) => failure(&request, cause),
    }
}

pub(super) async fn artifact(
    State(state): State<AppState>,
    Path((id, artifact_id)): Path<(String, String)>,
    request: Request,
) -> Response {
    let engine = match runtime(&state) {
        Ok(engine) => engine,
        Err(cause) => return failure(&request, cause),
    };
    let job = match engine.get(&id).await {
        Ok(job) => job,
        Err(cause) => return failure(&request, cause),
    };
    if !job
        .artifacts
        .iter()
        .any(|artifact| artifact.id == artifact_id)
    {
        return failure(&request, JobError::NotFound);
    }
    let display = job
        .artifacts
        .iter()
        .find(|artifact| artifact.id == artifact_id)
        .map(|artifact| {
            if artifact.filename.is_empty() {
                artifact.id.as_str()
            } else {
                artifact.filename.as_str()
            }
        })
        .unwrap_or(&artifact_id);
    // Worker filenames only contain safe ASCII basename bytes. Never use a path from a request.
    let disposition = HeaderValue::from_str(&format!("attachment; filename=\"{display}\""))
        .unwrap_or_else(|_| HeaderValue::from_static("attachment"));
    match engine.open_artifact(&id, &artifact_id).await {
        Ok((file, size)) => {
            let stream = futures_util::stream::try_unfold(
                (file, size),
                |(mut file, remaining)| async move {
                    use tokio::io::AsyncReadExt;
                    if remaining == 0 {
                        return Ok::<_, std::io::Error>(None);
                    }
                    let mut bytes = vec![0u8; remaining.min(65536) as usize];
                    let read = file.read(&mut bytes).await?;
                    if read == 0 {
                        return Err(std::io::Error::other("artifact_truncated"));
                    }
                    bytes.truncate(read);
                    Ok(Some((bytes, (file, remaining - read as u64))))
                },
            );
            (
                [
                    (
                        header::CONTENT_TYPE,
                        HeaderValue::from_static("application/octet-stream"),
                    ),
                    (
                        header::X_CONTENT_TYPE_OPTIONS,
                        HeaderValue::from_static("nosniff"),
                    ),
                    (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
                    (header::CONTENT_DISPOSITION, disposition),
                ],
                Body::from_stream(stream),
            )
                .into_response()
        }
        Err(cause) => failure(&request, cause),
    }
}
pub(super) async fn cancel(
    State(state): State<AppState>,
    Path(id): Path<String>,
    request: Request,
) -> Response {
    // Empty typed body rejects forged routing/security fields even on cancellation.
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Cancel {}
    let (request, _): (_, Cancel) = match json(request).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    match runtime(&state) {
        Ok(engine) => match engine.cancel(&id).await {
            Ok(job) => Json(job).into_response(),
            Err(e) => failure(&request, e),
        },
        Err(e) => failure(&request, e),
    }
}
pub(super) async fn retry(
    State(state): State<AppState>,
    Path(id): Path<String>,
    request: Request,
) -> Response {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Retry {
        confirm_safe_read_only: bool,
    }
    let key = match request.headers().get("Idempotency-Key") {
        Some(value) => match value.to_str() {
            Ok(value) => Some(value.to_owned()),
            Err(_) => return failure(&request, JobError::InvalidArguments),
        },
        None => None,
    };
    let (request, input): (_, Retry) = match json(request).await {
        Ok(value) => value,
        Err(error) => return error,
    };
    let engine = match runtime(&state) {
        Ok(engine) => engine,
        Err(error) => return failure(&request, error),
    };
    match engine
        .retry(&id, input.confirm_safe_read_only, key.as_deref())
        .await
    {
        Ok(receipt) => (
            if receipt.replayed {
                StatusCode::OK
            } else {
                StatusCode::CREATED
            },
            Json(receipt.job),
        )
            .into_response(),
        Err(error) => failure(&request, error),
    }
}

#[derive(Deserialize)]
pub(super) struct EventQuery {
    after: Option<i64>,
    limit: Option<u32>,
}
pub(super) async fn events(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<EventQuery>,
    request: Request,
) -> Response {
    let Some(after) = query.after.or(Some(0)).filter(|n| *n >= 0) else {
        return failure(&request, JobError::InvalidArguments);
    };
    let limit = query.limit.unwrap_or(100).clamp(1, 100);
    match runtime(&state) {
        Ok(engine) => match engine.database.job_events(&id, after, limit).await {
            Ok(items) => Json(serde_json::json!({"items":items})).into_response(),
            Err(err) => failure(&request, err),
        },
        Err(err) => failure(&request, err),
    }
}

/// In-memory invalidations only: consumers reconcile using authoritative REST reads.
pub(super) async fn stream(State(state): State<AppState>, request: Request) -> Response {
    use axum::response::sse::{Event, KeepAlive, Sse};
    use std::{convert::Infallible, time::Duration};
    use tokio::sync::broadcast::error::RecvError;

    let engine = match runtime(&state) {
        Ok(engine) => engine,
        Err(cause) => return failure(&request, cause),
    };
    // Subscribe before emitting ready so the initial REST read cannot race admission.
    let receiver = engine.database.job_changes.subscribe();
    let messages =
        futures_util::stream::unfold((receiver, true), |(mut receiver, first)| async move {
            if first {
                return Some((
                    Ok::<Event, Infallible>(Event::default().event("ready").data("{}")),
                    (receiver, false),
                ));
            }
            match receiver.recv().await {
                Ok(job_id) => Some((
                    Ok(Event::default()
                        .event("job_changed")
                        .data(serde_json::json!({ "job_id": job_id }).to_string())),
                    (receiver, false),
                )),
                Err(RecvError::Lagged(_)) => Some((
                    Ok(Event::default().event("resync_required").data("{}")),
                    (receiver, false),
                )),
                Err(RecvError::Closed) => None,
            }
        });
    let mut response = Sse::new(messages)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("keepalive"),
        )
        .into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-cache, no-transform"),
    );
    response
        .headers_mut()
        .insert("x-accel-buffering", HeaderValue::from_static("no"));
    response
}
