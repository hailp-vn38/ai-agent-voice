use super::{
    attachments::{
        JobAttachmentInput, JobAttachmentMediaKind, MAX_JOB_ATTACHMENT_BYTES, MAX_JOB_ATTACHMENTS,
        decode_attachment_base64, sanitize_job_attachment_name, valid_attachment_mime_for_kind,
    },
    task_failure, task_manager, task_manager_for_submit, *,
};
use crate::services::task_flow::{project_task_events, project_task_to_admin_job};
use orchestrator_flow_rs::{
    CreateTaskRequest, TaskEvent, TaskFeedItem, TaskFilter, TaskManager, TaskSource, TaskStatus,
};
use serde::Deserialize;

#[derive(Deserialize)]
pub(in crate::app::admin) struct CreateJobInput {
    pub goal: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub target_agent: Option<String>,
    #[serde(default)]
    pub related_wiki_links: Vec<String>,
    #[serde(default)]
    pub attachments: Vec<JobAttachmentInput>,
}

fn extract_idempotency_key(
    headers: &axum::http::HeaderMap,
) -> Result<Option<String>, &'static str> {
    let Some(raw) = headers.get("Idempotency-Key") else {
        return Ok(None);
    };
    let s = raw.to_str().map_err(|_| "invalid_arguments")?.trim();
    if s.is_empty() || s.len() > 128 || !s.is_ascii() {
        return Err("invalid_arguments");
    }
    Ok(Some(s.to_owned()))
}

async fn lookup_idempotent_job(
    state: &AppState,
    tm: &TaskManager,
    key: Option<&str>,
    fingerprint: &str,
) -> Result<Option<Value>, &'static str> {
    let Some(key) = key else {
        return Ok(None);
    };
    let existing = state
        .admin_task_control
        .lock()
        .expect("admin_task_control mutex poisoned")
        .get_idempotent(key);
    let Some((saved_fp, task_id)) = existing else {
        return Ok(None);
    };
    if saved_fp != fingerprint {
        return Err("idempotency_key_conflict");
    }
    if let Ok(task) = tm.get_task_any_scope(&task_id).await {
        let detail = tm.get_task_detail_any_scope(&task.id).await.ok();
        return Ok(Some(project_task_to_admin_job(&task, detail.as_ref())));
    }
    Ok(None)
}

fn store_idempotent_job(
    state: &AppState,
    key: Option<String>,
    fingerprint: String,
    task_id: String,
) {
    if let Some(key) = key {
        state
            .admin_task_control
            .lock()
            .expect("admin_task_control mutex poisoned")
            .insert_idempotent(key, fingerprint, task_id);
    }
}

pub(in crate::app::admin) async fn create(
    State(state): State<AppState>,
    request: Request,
) -> Response {
    use sha2::{Digest, Sha256};

    let idempotency_key = match extract_idempotency_key(request.headers()) {
        Ok(v) => v,
        Err(code) => return error(&request, StatusCode::BAD_REQUEST, code),
    };
    let (request, input): (_, CreateJobInput) =
        match json_with_limit(request, MAX_JOB_CREATE_BODY).await {
            Ok(v) => v,
            Err(e) => return e,
        };
    let tm = match task_manager_for_submit(&state, &request) {
        Ok(tm) => tm,
        Err(resp) => return resp,
    };
    let goal = input.goal.trim();
    if goal.is_empty() {
        return error(&request, StatusCode::BAD_REQUEST, "invalid_arguments");
    }
    if input.attachments.len() > MAX_JOB_ATTACHMENTS {
        return error(&request, StatusCode::BAD_REQUEST, "invalid_attachment");
    }

    let mut artifact_inputs = Vec::with_capacity(input.attachments.len());
    let mut attachment_digests = Vec::with_capacity(input.attachments.len());
    for att in input.attachments {
        let Some((clean_name, media_kind)) = sanitize_job_attachment_name(&att.name) else {
            return error(&request, StatusCode::BAD_REQUEST, "invalid_attachment");
        };
        if !valid_attachment_mime_for_kind(&att.mime_type, media_kind) {
            return error(&request, StatusCode::BAD_REQUEST, "invalid_attachment");
        }
        let bytes = match decode_attachment_base64(&att.data_base64) {
            Ok(b) => b,
            Err(code) => return error(&request, StatusCode::BAD_REQUEST, code),
        };
        if bytes.len() > MAX_JOB_ATTACHMENT_BYTES {
            return error(
                &request,
                StatusCode::PAYLOAD_TOO_LARGE,
                "attachment_too_large",
            );
        }
        let header_ok = match media_kind {
            JobAttachmentMediaKind::Wav => orchestrator_flow_rs::inspect_wav_bytes(&bytes).is_ok(),
            JobAttachmentMediaKind::Mp4 => orchestrator_flow_rs::inspect_mp4_bytes(&bytes).is_ok(),
        };
        if !header_ok {
            return error(&request, StatusCode::BAD_REQUEST, "invalid_attachment");
        }
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let sha_hex: String = hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        attachment_digests.push(format!("{clean_name}|{sha_hex}"));
        artifact_inputs.push(orchestrator_agent_rs::ArtifactInput::intermediate_bytes(
            clean_name,
            media_kind.canonical_mime(),
            bytes,
        ));
    }

    let title = input
        .title
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| goal.chars().take(80).collect());
    let target_agent = input
        .target_agent
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    let mut fingerprint = format!(
        "create:{title}:{goal}:{}:{:?}",
        target_agent.as_deref().unwrap_or(""),
        input.related_wiki_links
    );
    if !attachment_digests.is_empty() {
        fingerprint.push_str(":att=");
        fingerprint.push_str(&attachment_digests.join(","));
    }

    match lookup_idempotent_job(&state, &tm, idempotency_key.as_deref(), &fingerprint).await {
        Ok(Some(job_json)) => return (StatusCode::OK, Json(job_json)).into_response(),
        Ok(None) => {}
        Err(code) => return error(&request, StatusCode::CONFLICT, code),
    }

    let mut req = CreateTaskRequest::new(title, goal)
        .with_source(TaskSource::Api)
        .with_wiki_links(input.related_wiki_links);
    if let Some(target) = target_agent {
        req = req.with_target(target);
    }
    if !artifact_inputs.is_empty() {
        req = req.with_input_artifacts(artifact_inputs);
    }

    match tm.create_task(req).await {
        Ok(task) => {
            store_idempotent_job(&state, idempotency_key, fingerprint, task.id.clone());
            let detail = tm.get_task_detail_any_scope(&task.id).await.ok();
            let job_json = project_task_to_admin_job(&task, detail.as_ref());
            (StatusCode::CREATED, Json(job_json)).into_response()
        }
        Err(e) => task_failure(&request, e),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::app::admin) struct JobQuery {
    page: Option<u32>,
    page_size: Option<u32>,
    status: Option<String>,
    q: Option<String>,
}

fn parse_status_filter(status: Option<&str>) -> Option<TaskStatus> {
    match status?.trim() {
        "queued" => Some(TaskStatus::Queued),
        "running" | "planning" | "cancelling" => Some(TaskStatus::Running),
        "succeeded" | "completed" | "partial_success" => Some(TaskStatus::Completed),
        "failed" => Some(TaskStatus::Failed),
        "cancelled" => Some(TaskStatus::Cancelled),
        "interrupted" => Some(TaskStatus::Interrupted),
        _ => None,
    }
}

pub(in crate::app::admin) async fn list(
    State(state): State<AppState>,
    Query(query): Query<JobQuery>,
    request: Request,
) -> Response {
    let page = query.page.unwrap_or(1).max(1);
    let size = query.page_size.unwrap_or(30).clamp(1, 200);
    let Some(tm) = state.task_manager.as_ref() else {
        return Json(serde_json::json!({
            "items": [],
            "total": 0,
            "page": page,
            "page_size": size,
        }))
        .into_response();
    };

    let offset = (page - 1).saturating_mul(size);
    let filter = TaskFilter {
        status: parse_status_filter(query.status.as_deref()),
        query: query.q,
        limit: Some(size),
        offset: Some(offset),
        ..Default::default()
    };

    match tm.list_all_tasks(&filter).await {
        Ok(task_page) => {
            let items: Vec<Value> = task_page
                .items
                .iter()
                .map(|task| project_task_to_admin_job(task, None))
                .collect();
            Json(serde_json::json!({
                "items": items,
                "total": task_page.total,
                "page": page,
                "page_size": size,
            }))
            .into_response()
        }
        Err(e) => task_failure(&request, e),
    }
}

pub(in crate::app::admin) async fn get(
    State(state): State<AppState>,
    Path(id): Path<String>,
    request: Request,
) -> Response {
    let tm = match task_manager(&state, &request) {
        Ok(tm) => tm,
        Err(resp) => return resp,
    };
    match tm.get_task_detail_any_scope(&id).await {
        Ok(detail) => Json(project_task_to_admin_job(&detail.task, Some(&detail))).into_response(),
        Err(e) => task_failure(&request, e),
    }
}

pub(in crate::app::admin) async fn delete(
    State(state): State<AppState>,
    Path(id): Path<String>,
    request: Request,
) -> Response {
    let tm = match task_manager(&state, &request) {
        Ok(tm) => tm,
        Err(resp) => return resp,
    };
    match tm.delete_task_any_scope(&id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => task_failure(&request, e),
    }
}

pub(in crate::app::admin) async fn cancel(
    State(state): State<AppState>,
    Path(id): Path<String>,
    request: Request,
) -> Response {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Cancel {}
    let (request, _): (_, Cancel) = match json(request).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    let tm = match task_manager(&state, &request) {
        Ok(tm) => tm,
        Err(resp) => return resp,
    };
    match tm.cancel_task_any_scope(&id).await {
        Ok(task) => {
            let detail = tm.get_task_detail_any_scope(&task.id).await.ok();
            Json(project_task_to_admin_job(&task, detail.as_ref())).into_response()
        }
        Err(e) => task_failure(&request, e),
    }
}

pub(in crate::app::admin) async fn retry(
    State(state): State<AppState>,
    Path(id): Path<String>,
    request: Request,
) -> Response {
    #[derive(Deserialize)]
    struct Retry {}
    let idempotency_key = match extract_idempotency_key(request.headers()) {
        Ok(v) => v,
        Err(code) => return error(&request, StatusCode::BAD_REQUEST, code),
    };
    let (request, _input): (_, Retry) = match json(request).await {
        Ok(value) => value,
        Err(error) => return error,
    };
    let tm = match task_manager_for_submit(&state, &request) {
        Ok(tm) => tm,
        Err(resp) => return resp,
    };
    let fingerprint = format!("retry:{id}");
    match lookup_idempotent_job(&state, &tm, idempotency_key.as_deref(), &fingerprint).await {
        Ok(Some(mut job_json)) => {
            job_json["retry_of_job_id"] = serde_json::json!(id);
            return (StatusCode::OK, Json(job_json)).into_response();
        }
        Ok(None) => {}
        Err(code) => return error(&request, StatusCode::CONFLICT, code),
    }
    let original = match tm.get_task_any_scope(&id).await {
        Ok(task) => task,
        Err(e) => return task_failure(&request, e),
    };
    let req = CreateTaskRequest::new(original.title, original.prompt)
        .with_target(format!("{}/{}", original.flow_id, original.agent_id))
        .with_source(TaskSource::Api)
        .with_wiki_links(original.related_wiki_links);
    match tm.create_task(req).await {
        Ok(task) => {
            store_idempotent_job(&state, idempotency_key, fingerprint, task.id.clone());
            let detail = tm.get_task_detail_any_scope(&task.id).await.ok();
            let mut job_json = project_task_to_admin_job(&task, detail.as_ref());
            job_json["retry_of_job_id"] = serde_json::json!(id);
            (StatusCode::CREATED, Json(job_json)).into_response()
        }
        Err(e) => task_failure(&request, e),
    }
}

#[derive(Deserialize)]
pub(in crate::app::admin) struct FollowupInput {
    pub prompt: String,
}

pub(in crate::app::admin) async fn followup(
    State(state): State<AppState>,
    Path(id): Path<String>,
    request: Request,
) -> Response {
    let (request, input): (_, FollowupInput) = match json(request).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    let tm = match task_manager(&state, &request) {
        Ok(tm) => tm,
        Err(resp) => return resp,
    };
    match tm
        .send_followup_any_scope(&id, input.prompt, TaskSource::Api)
        .await
    {
        Ok(followup_view) => (StatusCode::ACCEPTED, Json(followup_view)).into_response(),
        Err(e) => task_failure(&request, e),
    }
}

#[derive(Deserialize)]
pub(in crate::app::admin) struct HistoryQuery {
    agent: Option<String>,
    limit: Option<u32>,
}

pub(in crate::app::admin) async fn history(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<HistoryQuery>,
    request: Request,
) -> Response {
    let tm = match task_manager(&state, &request) {
        Ok(tm) => tm,
        Err(resp) => return resp,
    };
    match tm
        .get_task_agent_history_any_scope(&id, query.agent.as_deref(), query.limit)
        .await
    {
        Ok(items) => Json(serde_json::json!({ "items": items })).into_response(),
        Err(e) => task_failure(&request, e),
    }
}

#[derive(Deserialize)]
pub(in crate::app::admin) struct EventQuery {
    after: Option<i64>,
    limit: Option<u32>,
}

pub(in crate::app::admin) async fn events(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<EventQuery>,
    request: Request,
) -> Response {
    let Some(after) = query.after.or(Some(0)).filter(|n| *n >= 0) else {
        return error(&request, StatusCode::BAD_REQUEST, "invalid_arguments");
    };
    let limit = query.limit.unwrap_or(100).clamp(1, 100);
    let tm = match task_manager(&state, &request) {
        Ok(tm) => tm,
        Err(resp) => return resp,
    };
    match tm.get_task_detail_any_scope(&id).await {
        Ok(detail) => {
            let items = project_task_events(&detail, after, limit);
            Json(serde_json::json!({ "items": items })).into_response()
        }
        Err(e) => task_failure(&request, e),
    }
}

fn task_id_from_event(event: &TaskEvent) -> Option<String> {
    match event {
        TaskEvent::TaskCreated { task } | TaskEvent::TaskTerminal { task } => Some(task.id.clone()),
        TaskEvent::TaskProgress { task_id, .. }
        | TaskEvent::TaskDeleted { task_id, .. }
        | TaskEvent::FollowupQueued { task_id, .. } => Some(task_id.clone()),
        TaskEvent::ScheduleTriggered {
            spawned_task_id, ..
        } => Some(spawned_task_id.clone()),
        TaskEvent::TaskExecutionStream {
            task_id,
            state_revision: Some(_),
            ..
        } => Some(task_id.clone()),
        TaskEvent::TaskExecutionStream { .. } | TaskEvent::ReminderDue { .. } => None,
    }
}

/// Real-time SSE stream backed by [`TaskManager::subscribe_all`].
pub(in crate::app::admin) async fn stream(
    State(state): State<AppState>,
    request: Request,
) -> Response {
    use axum::response::sse::{Event, KeepAlive, Sse};
    use std::{convert::Infallible, time::Duration};

    let tm = match task_manager(&state, &request) {
        Ok(tm) => tm,
        Err(resp) => return resp,
    };
    let subscription = match tm.subscribe_all() {
        Ok(sub) => sub,
        Err(e) => return task_failure(&request, e),
    };
    let stopping = state.lifecycle.stopping().clone();

    let messages = futures_util::stream::unfold(
        (subscription, stopping, true),
        |(mut sub, stopping, first)| async move {
            if first {
                return Some((
                    Ok::<Event, Infallible>(Event::default().event("ready").data("{}")),
                    (sub, stopping, false),
                ));
            }
            loop {
                let item = tokio::select! {
                    biased;
                    _ = stopping.cancelled() => return None,
                    item = sub.recv() => item?,
                };
                match item {
                    TaskFeedItem::Event(ev) => {
                        if let Some(job_id) = task_id_from_event(&ev) {
                            return Some((
                                Ok(Event::default()
                                    .event("job_changed")
                                    .data(serde_json::json!({ "job_id": job_id }).to_string())),
                                (sub, stopping, false),
                            ));
                        }
                    }
                    TaskFeedItem::ResyncRequired => {
                        return Some((
                            Ok(Event::default().event("resync_required").data("{}")),
                            (sub, stopping, false),
                        ));
                    }
                }
            }
        },
    );

    let mut response = Sse::new(messages)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("keepalive"),
        )
        .into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}
