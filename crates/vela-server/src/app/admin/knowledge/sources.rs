use super::{trigger_source_analysis, wiki, wiki_error, *};
use llm_wiki::{GetOptions, MultiGetOptions, WriteTopicSourceRequest, slugify};
use serde::Deserialize;

pub(in crate::app::admin) async fn knowledge_sources(
    State(state): State<AppState>,
    req: Request,
) -> Response {
    let engine = match wiki(&state, &req) {
        Ok(engine) => engine,
        Err(resp) => return resp,
    };
    match engine.multi_get("sources/**/*.md", MultiGetOptions::default()) {
        Ok(batch) => {
            let items: Vec<Value> = batch
                .docs
                .into_iter()
                .map(|doc| {
                    serde_json::json!({
                        "id": doc.info.rel_path,
                        "docid": doc.info.docid,
                        "version": 1,
                        "name": doc.info.title,
                        "provenance": doc.info.rel_path,
                        "sha256": doc.info.content_hash,
                        "size_bytes": doc.info.byte_size,
                        "media_type": "text/markdown",
                        "rel_path": doc.info.rel_path,
                    })
                })
                .collect();
            Json(serde_json::json!({ "items": items })).into_response()
        }
        Err(err) => wiki_error(&req, err),
    }
}

#[derive(Deserialize)]
pub(in crate::app::admin) struct SourceImportPayload {
    pub name: String,
    pub content: String,
    #[serde(default)]
    pub provenance: Option<String>,
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default)]
    pub topic: Option<String>,
}

pub(in crate::app::admin) async fn knowledge_import_source(
    State(state): State<AppState>,
    req: Request,
) -> Response {
    let (req, input): (_, SourceImportPayload) = match json(req).await {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let engine = match wiki(&state, &req) {
        Ok(engine) => engine,
        Err(resp) => return resp,
    };
    if input.name.trim().is_empty() || input.content.trim().is_empty() {
        return error(&req, StatusCode::BAD_REQUEST, "invalid_arguments");
    }
    let domain = input.domain.unwrap_or_else(|| "general".into());
    let topic = input.topic.unwrap_or_else(|| "notes".into());
    let mut header_bullets = Vec::new();
    if let Some(ref prov) = input.provenance {
        header_bullets.push(("Provenance".to_string(), prov.clone()));
    }
    let write_req = WriteTopicSourceRequest {
        topic_folder: format!("{domain}/{topic}"),
        file_stem: Some(slugify(&input.name, 64)),
        title: input.name.clone(),
        raw_body: input.content.clone(),
        body_heading: Some("Raw Content".into()),
        tags: Vec::new(),
        extra_frontmatter: Vec::new(),
        header_bullets,
        preview_chars: Some(400),
    };
    match engine.write_topic_source(write_req) {
        Ok(saved) => {
            let mut body = serde_json::json!({
                "id": saved.info.rel_path,
                "docid": saved.info.docid,
                "version": 1,
                "name": saved.info.title,
                "provenance": input.provenance.unwrap_or_else(|| saved.info.rel_path.clone()),
                "sha256": saved.info.content_hash,
                "size_bytes": saved.info.byte_size,
                "media_type": "text/markdown",
                "rel_path": saved.info.rel_path,
                "expected_wiki_path": saved.expected_wiki_path,
            });
            trigger_source_analysis(&state, &saved.info.rel_path, &mut body).await;
            (StatusCode::CREATED, Json(body)).into_response()
        }
        Err(err) => wiki_error(&req, err),
    }
}

pub(in crate::app::admin) async fn knowledge_read_source(
    State(state): State<AppState>,
    Path(id): Path<String>,
    req: Request,
) -> Response {
    let engine = match wiki(&state, &req) {
        Ok(engine) => engine,
        Err(resp) => return resp,
    };
    match engine.get_document(&id, GetOptions::default()) {
        Ok(doc) => Json(serde_json::json!({
            "text": doc.content,
            "info": doc.info,
        }))
        .into_response(),
        Err(err) => wiki_error(&req, err),
    }
}

pub(in crate::app::admin) async fn knowledge_chart_read(
    State(state): State<AppState>,
    Path(path): Path<String>,
    req: Request,
) -> Response {
    let engine = match wiki(&state, &req) {
        Ok(engine) => engine,
        Err(resp) => return resp,
    };
    let vault_dir = &engine.config().data_root;
    match crate::services::task_flow::read_chart_json_for_admin(vault_dir, &path) {
        Ok(doc) => Json(doc).into_response(),
        Err(err) => match err.code() {
            "chart_not_found" => error(&req, StatusCode::NOT_FOUND, "not_found"),
            "invalid_chart_path" => error(&req, StatusCode::BAD_REQUEST, "invalid_path"),
            _ => error(&req, StatusCode::BAD_REQUEST, err.code()),
        },
    }
}
