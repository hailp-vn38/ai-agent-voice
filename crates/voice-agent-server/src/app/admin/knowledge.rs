//! Authenticated Admin adapter: never writes filesystem or index tables directly.
use super::*;
use crate::services::knowledge::{
    Error as KnowledgeError, FactInput, Runtime, SourceInput, WikiInput,
};
use std::sync::Arc;

async fn runtime(state: &AppState) -> Result<Arc<Runtime>, KnowledgeError> {
    let data_root = state
        .database
        .as_ref()
        .and_then(|db| db.durable_path())
        .and_then(|path| path.parent())
        .ok_or(KnowledgeError::Unavailable)?
        .to_path_buf();
    let core = state
        .knowledge_runtime
        .get_or_try_init(|| Runtime::open(data_root))
        .await?;
    Ok(Arc::clone(core))
}
fn failure(req: &Request, e: KnowledgeError) -> Response {
    let code = e.code();
    let status = match e {
        KnowledgeError::InvalidInput | KnowledgeError::InvalidPath => StatusCode::BAD_REQUEST,
        KnowledgeError::NotFound => StatusCode::NOT_FOUND,
        KnowledgeError::RevisionConflict => StatusCode::CONFLICT,
        KnowledgeError::IndexStale => StatusCode::SERVICE_UNAVAILABLE,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    };
    error(req, status, code)
}
#[derive(Deserialize)]
pub(super) struct SearchQuery {
    q: String,
    limit: Option<i64>,
}
#[derive(Deserialize)]
pub(super) struct PageLines {
    from_line: Option<usize>,
    max_lines: Option<usize>,
}

pub(super) async fn knowledge_health(State(state): State<AppState>, req: Request) -> Response {
    match runtime(&state).await {
        Ok(core) => match core.knowledge.health().await {
            Ok(info) => Json(info).into_response(),
            Err(e) => failure(&req, e),
        },
        Err(e) => failure(&req, e),
    }
}
pub(super) async fn knowledge_rebuild(State(state): State<AppState>, req: Request) -> Response {
    match runtime(&state).await {
        Ok(core) => match core.knowledge.rebuild().await {
            Ok(indexed) => Json(serde_json::json!({"indexed":indexed})).into_response(),
            Err(e) => failure(&req, e),
        },
        Err(e) => failure(&req, e),
    }
}
pub(super) async fn knowledge_search(
    State(state): State<AppState>,
    Query(q): Query<SearchQuery>,
    req: Request,
) -> Response {
    match runtime(&state).await {
        Ok(core) => match core
            .knowledge
            .admin_search(&q.q, q.limit.unwrap_or(20))
            .await
        {
            Ok(items) => Json(serde_json::json!({"items":items})).into_response(),
            Err(e) => failure(&req, e),
        },
        Err(e) => failure(&req, e),
    }
}
pub(super) async fn knowledge_read(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(lines): Query<PageLines>,
    req: Request,
) -> Response {
    match runtime(&state).await {
        Ok(core) => match core
            .knowledge
            .read(
                &id,
                lines.from_line.unwrap_or(1),
                lines.max_lines.unwrap_or(200),
            )
            .await
        {
            Ok(page) => Json(page).into_response(),
            Err(e) => failure(&req, e),
        },
        Err(e) => failure(&req, e),
    }
}
pub(super) async fn knowledge_create(State(state): State<AppState>, req: Request) -> Response {
    let (req, mut input): (_, WikiInput) = match json(req).await {
        Ok(x) => x,
        Err(e) => return e,
    };
    if input.id.is_some() || input.expected_revision.is_some_and(|r| r != 0) {
        return error(&req, StatusCode::BAD_REQUEST, "invalid_arguments");
    }
    input.expected_revision = Some(0);
    match runtime(&state).await {
        Ok(core) => match core.knowledge.put_draft(input).await {
            Ok(page) => (StatusCode::CREATED, Json(page)).into_response(),
            Err(e) => failure(&req, e),
        },
        Err(e) => failure(&req, e),
    }
}
pub(super) async fn knowledge_update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    req: Request,
) -> Response {
    let (req, mut input): (_, WikiInput) = match json(req).await {
        Ok(x) => x,
        Err(e) => return e,
    };
    if input.id.as_ref().is_some_and(|v| v != &id) {
        return error(&req, StatusCode::BAD_REQUEST, "invalid_arguments");
    }
    input.id = Some(id);
    match runtime(&state).await {
        Ok(core) => match core.knowledge.put_draft(input).await {
            Ok(page) => Json(page).into_response(),
            Err(e) => failure(&req, e),
        },
        Err(e) => failure(&req, e),
    }
}
pub(super) async fn memory_health(State(state): State<AppState>, req: Request) -> Response {
    match runtime(&state).await {
        Ok(core) => match core.memory.health().await {
            Ok(info) => Json(info).into_response(),
            Err(e) => failure(&req, e),
        },
        Err(e) => failure(&req, e),
    }
}
pub(super) async fn memory_rebuild(State(state): State<AppState>, req: Request) -> Response {
    match runtime(&state).await {
        Ok(core) => match core.memory.rebuild().await {
            Ok(indexed) => Json(serde_json::json!({"indexed":indexed})).into_response(),
            Err(e) => failure(&req, e),
        },
        Err(e) => failure(&req, e),
    }
}
#[derive(Deserialize)]
pub(super) struct MemoryQuery {
    q: Option<String>,
    limit: Option<i64>,
}
pub(super) async fn memory_recall(
    State(state): State<AppState>,
    Query(q): Query<MemoryQuery>,
    req: Request,
) -> Response {
    match runtime(&state).await {
        Ok(core) => {
            if let Some(query) = q.q.as_deref().filter(|q| !q.trim().is_empty()) {
                match core.memory.recall(query, q.limit.unwrap_or(20)).await {
                    Ok(items) => Json(serde_json::json!({"items":items})).into_response(),
                    Err(e) => failure(&req, e),
                }
            } else {
                let limit = q.limit.unwrap_or(100);
                let Ok(limit) = usize::try_from(limit) else {
                    return error(&req, StatusCode::BAD_REQUEST, "invalid_arguments");
                };
                match core.memory.admin_list(limit).await {
                    Ok(items) => Json(serde_json::json!({"items":items})).into_response(),
                    Err(e) => failure(&req, e),
                }
            }
        }
        Err(e) => failure(&req, e),
    }
}
pub(super) async fn memory_summary(State(state): State<AppState>, req: Request) -> Response {
    match runtime(&state).await {
        Ok(core) => match core.memory.summary().await {
            Ok(summary) => Json(serde_json::json!({"summary":summary})).into_response(),
            Err(e) => failure(&req, e),
        },
        Err(e) => failure(&req, e),
    }
}
pub(super) async fn memory_read(
    State(state): State<AppState>,
    Path(id): Path<String>,
    req: Request,
) -> Response {
    match runtime(&state).await {
        Ok(core) => match core.memory.read(&id).await {
            Ok(fact) => Json(fact).into_response(),
            Err(e) => failure(&req, e),
        },
        Err(e) => failure(&req, e),
    }
}
pub(super) async fn memory_create(State(state): State<AppState>, req: Request) -> Response {
    let (req, mut input): (_, FactInput) = match json(req).await {
        Ok(x) => x,
        Err(e) => return e,
    };
    if input.id.is_some() || input.expected_revision.is_some_and(|r| r != 0) {
        return error(&req, StatusCode::BAD_REQUEST, "invalid_arguments");
    }
    input.expected_revision = Some(0);
    match runtime(&state).await {
        Ok(core) => match core.memory.put_fact(input).await {
            Ok(fact) => (StatusCode::CREATED, Json(fact)).into_response(),
            Err(e) => failure(&req, e),
        },
        Err(e) => failure(&req, e),
    }
}
pub(super) async fn memory_update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    req: Request,
) -> Response {
    let (req, mut input): (_, FactInput) = match json(req).await {
        Ok(x) => x,
        Err(e) => return e,
    };
    if input.id.as_ref().is_some_and(|v| v != &id) {
        return error(&req, StatusCode::BAD_REQUEST, "invalid_arguments");
    }
    input.id = Some(id);
    match runtime(&state).await {
        Ok(core) => match core.memory.put_fact(input).await {
            Ok(fact) => Json(fact).into_response(),
            Err(e) => failure(&req, e),
        },
        Err(e) => failure(&req, e),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Forget {
    expected_revision: i64,
}
pub(super) async fn memory_forget(
    State(state): State<AppState>,
    Path(id): Path<String>,
    req: Request,
) -> Response {
    let (req, input): (_, Forget) = match json(req).await {
        Ok(x) => x,
        Err(e) => return e,
    };
    match runtime(&state).await {
        Ok(core) => match core.memory.forget(&id, input.expected_revision).await {
            Ok(fact) => Json(fact).into_response(),
            Err(e) => failure(&req, e),
        },
        Err(e) => failure(&req, e),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PublishInput {
    expected_revision: i64,
    source_ref: String,
}
pub(super) async fn knowledge_publish(
    State(state): State<AppState>,
    Path(id): Path<String>,
    req: Request,
) -> Response {
    let (req, input): (_, PublishInput) = match json(req).await {
        Ok(x) => x,
        Err(e) => return e,
    };
    match runtime(&state).await {
        Ok(core) => match core
            .knowledge
            .publish(&id, input.expected_revision, &input.source_ref)
            .await
        {
            Ok(page) => Json(page).into_response(),
            Err(e) => failure(&req, e),
        },
        Err(e) => failure(&req, e),
    }
}
pub(super) async fn knowledge_sources(State(state): State<AppState>, req: Request) -> Response {
    match runtime(&state).await {
        Ok(core) => match core.knowledge.list_sources().await {
            Ok(items) => Json(serde_json::json!({"items":items})).into_response(),
            Err(e) => failure(&req, e),
        },
        Err(e) => failure(&req, e),
    }
}
pub(super) async fn knowledge_import_source(
    State(state): State<AppState>,
    req: Request,
) -> Response {
    let (req, input): (_, SourceInput) = match json(req).await {
        Ok(x) => x,
        Err(e) => return e,
    };
    match runtime(&state).await {
        Ok(core) => match core.knowledge.import_source(input).await {
            Ok(source) => (StatusCode::CREATED, Json(source)).into_response(),
            Err(e) => failure(&req, e),
        },
        Err(e) => failure(&req, e),
    }
}
pub(super) async fn knowledge_read_source(
    State(state): State<AppState>,
    Path(id): Path<String>,
    req: Request,
) -> Response {
    match runtime(&state).await {
        Ok(core) => match core.knowledge.read_source(&id).await {
            Ok(text) => Json(serde_json::json!({"text":text})).into_response(),
            Err(e) => failure(&req, e),
        },
        Err(e) => failure(&req, e),
    }
}
