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
        JobError::RevisionConflict | JobError::IdempotencyConflict | JobError::Terminal => {
            StatusCode::CONFLICT
        }
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
