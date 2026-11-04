//! Physical folder tree & Raw/Wiki file browser handlers for `/api/admin/knowledge/*`.

use super::{resolve_wiki_scoped_path, sha256_hex, trigger_source_analysis, wiki, wiki_error};
use crate::app::{
    AppState,
    admin::{
        Json, Path, Query, Request, Response, State, StatusCode, error, json, json_with_limit,
    },
};
use axum::response::IntoResponse;
use llm_wiki::{
    CollectionScope, FileTreeNode, GetOptions, NodeType, TreeOptions, TreeScope, WikiEngine,
    WikiError, slugify,
};
use serde::{Deserialize, Serialize};

/// Strips the UI space prefix (`raw`, `sources` or `wiki`) from an Admin path, including the
/// bare root folder itself, so `"raw"` and `"raw/notes"` resolve to `""` and `"notes"`.
fn space_relative(path: &str) -> &str {
    let path = path.trim().trim_matches('/');
    for space in ["sources", "raw", "wiki"] {
        if path == space {
            return "";
        }
        if let Some(rest) = path
            .strip_prefix(space)
            .and_then(|rest| rest.strip_prefix('/'))
        {
            return rest;
        }
    }
    path
}

fn verify_expected_sha256(
    engine: &WikiEngine,
    normalized_path: &str,
    expected_sha256: Option<&str>,
) -> bool {
    let Some(expected) = expected_sha256.map(str::trim).filter(|s| !s.is_empty()) else {
        return true;
    };
    let Ok(existing) = engine.get_document(normalized_path, GetOptions::default()) else {
        return true;
    };
    let current_sha = sha256_hex(existing.content.as_bytes());
    current_sha.eq_ignore_ascii_case(expected)
        || existing.info.content_hash.eq_ignore_ascii_case(expected)
}

#[derive(Deserialize)]
pub(in crate::app::admin) struct RawPathQuery {
    path: String,
}

#[derive(Deserialize)]
pub(in crate::app::admin) struct KnowledgeTreeQuery {
    space: Option<String>,
}

#[derive(Serialize)]
struct FlatTreeNode {
    #[serde(rename = "type")]
    node_type: String,
    space: String,
    path: String,
    name: String,
    editable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    revision: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<String>,
}

fn flatten_tree_nodes(node: &FileTreeNode, default_space: &str, out: &mut Vec<FlatTreeNode>) {
    for child in &node.children {
        let space = if child.vault_rel_path.starts_with("sources") {
            "raw"
        } else if child.vault_rel_path.starts_with("wiki") {
            "wiki"
        } else {
            default_space
        };
        let ui_path = if space == "raw" && child.vault_rel_path.starts_with("sources/") {
            child.vault_rel_path.replacen("sources/", "raw/", 1)
        } else if space == "raw" && child.vault_rel_path == "sources" {
            "raw".to_string()
        } else {
            child.vault_rel_path.clone()
        };
        let is_dir = child.node_type == NodeType::Directory;
        out.push(FlatTreeNode {
            node_type: if is_dir {
                "folder".into()
            } else {
                "file".into()
            },
            space: space.into(),
            path: ui_path,
            name: child.name.clone(),
            editable: true,
            id: if !is_dir {
                Some(child.vault_rel_path.clone())
            } else {
                None
            },
            revision: if !is_dir { Some(1) } else { None },
            status: if !is_dir {
                Some("verified".into())
            } else {
                None
            },
        });
        if is_dir {
            flatten_tree_nodes(child, space, out);
        }
    }
}

pub(in crate::app::admin) async fn knowledge_tree(
    State(state): State<AppState>,
    Query(q): Query<KnowledgeTreeQuery>,
    req: Request,
) -> Response {
    let engine = match wiki(&state, &req) {
        Ok(engine) => engine,
        Err(resp) => return resp,
    };
    if q.space.as_deref() == Some("drafts") {
        return Json(serde_json::json!({ "items": [] })).into_response();
    }
    let (scope, default_space) = match q.space.as_deref() {
        Some("raw" | "sources" | "source") => (TreeScope::Sources, "raw"),
        Some("wiki") => (TreeScope::Wiki, "wiki"),
        _ => (TreeScope::All, "wiki"),
    };
    match engine.get_tree(TreeOptions::for_scope(scope)) {
        Ok(tree) => {
            let mut items = Vec::new();
            flatten_tree_nodes(&tree, default_space, &mut items);
            Json(serde_json::json!({
                "items": items,
                "tree": tree,
            }))
            .into_response()
        }
        Err(err) => wiki_error(&req, err),
    }
}

#[derive(Deserialize)]
struct FolderCreatePayload {
    space: String,
    parent_path: String,
    name: String,
}

pub(in crate::app::admin) async fn knowledge_folder_create(
    State(state): State<AppState>,
    req: Request,
) -> Response {
    let (req, input): (_, FolderCreatePayload) = match json(req).await {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let engine = match wiki(&state, &req) {
        Ok(engine) => engine,
        Err(resp) => return resp,
    };
    let base_dir = match input.space.as_str() {
        "raw" | "sources" => engine.config().sources_dir(),
        "wiki" => engine.config().wiki_dir(),
        _ => return error(&req, StatusCode::BAD_REQUEST, "invalid_arguments"),
    };
    let clean_parent = space_relative(&input.parent_path);
    let clean_name = input
        .name
        .split('/')
        .filter(|s| !s.trim().is_empty())
        .map(|s| slugify(s, 64))
        .collect::<Vec<_>>()
        .join("/");
    if clean_name.is_empty() {
        return error(&req, StatusCode::BAD_REQUEST, "invalid_arguments");
    }
    let rel = if clean_parent.is_empty() {
        clean_name.clone()
    } else {
        format!("{clean_parent}/{clean_name}")
    };
    let target = base_dir.join(&rel);
    if let Err(err) = std::fs::create_dir_all(&target) {
        return wiki_error(&req, WikiError::Io(err));
    }
    let ui_space = if input.space == "wiki" { "wiki" } else { "raw" };
    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "type": "folder",
            "space": ui_space,
            "path": format!("{ui_space}/{rel}"),
            "name": clean_name,
            "editable": true,
        })),
    )
        .into_response()
}

#[derive(Deserialize)]
struct RawCreatePayload {
    folder_path: String,
    file_name: String,
    body: String,
}

pub(in crate::app::admin) async fn knowledge_raw_create(
    State(state): State<AppState>,
    req: Request,
) -> Response {
    let (req, input): (_, RawCreatePayload) = match json(req).await {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let engine = match wiki(&state, &req) {
        Ok(engine) => engine,
        Err(resp) => return resp,
    };
    let folder = space_relative(&input.folder_path);
    let file_name = input.file_name.trim().trim_matches('/');
    if file_name.is_empty() {
        return error(&req, StatusCode::BAD_REQUEST, "invalid_arguments");
    }
    let rel_path = if folder.is_empty() {
        file_name.to_string()
    } else {
        format!("{folder}/{file_name}")
    };
    let md_path = if rel_path.to_lowercase().ends_with(".md") {
        rel_path.clone()
    } else {
        format!("{rel_path}.md")
    };
    // Creating never replaces an existing note; edits go through the revision-checked update.
    if engine.config().sources_dir().join(&md_path).exists() {
        return error(&req, StatusCode::CONFLICT, "file_exists");
    }
    match engine.write_source_file(&rel_path, &input.body) {
        Ok(info) => {
            if let Some(ref notify) = state.autoflow_notify {
                notify.notify_one();
            }
            (
                StatusCode::CREATED,
                Json(serde_json::json!({
                    "path": info.rel_path.replacen("sources/", "raw/", 1),
                    "docid": info.docid,
                    "body": input.body,
                    "sha256": sha256_hex(input.body.as_bytes()),
                })),
            )
                .into_response()
        }
        Err(err) => wiki_error(&req, err),
    }
}

/// Tổng dung lượng tối đa của một request upload nhiều file (UI tự chia nhỏ theo ngưỡng này).
pub(in crate::app::admin) const MAX_RAW_BATCH_BODY: usize = 8 * 1024 * 1024;
const MAX_RAW_BATCH_FILES: usize = 200;

#[derive(Deserialize)]
struct RawBatchFile {
    /// Đường dẫn tương đối trong folder đích, có thể lồng nhau (`guide/setup.md`).
    path: String,
    body: String,
}

#[derive(Deserialize)]
struct RawBatchPayload {
    folder_path: String,
    files: Vec<RawBatchFile>,
}

/// Chuẩn hóa đường dẫn tương đối của một file upload; từ chối `..`, đường dẫn rỗng và file
/// không phải Markdown.
fn batch_rel_path(folder: &str, path: &str) -> Option<String> {
    let segments: Vec<&str> = path
        .split(['/', '\\'])
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != ".")
        .collect();
    if segments.is_empty() || segments.iter().any(|s| *s == ".." || s.starts_with('.')) {
        return None;
    }
    let joined = segments.join("/");
    if !joined.to_lowercase().ends_with(".md") {
        return None;
    }
    Some(if folder.is_empty() {
        joined
    } else {
        format!("{folder}/{joined}")
    })
}

/// Tạo nhiều file nguồn một lần (upload cả folder). File đã tồn tại được bỏ qua, không ghi đè;
/// Auto-Flow chỉ được báo một lần cho cả lô.
pub(in crate::app::admin) async fn knowledge_raw_batch_create(
    State(state): State<AppState>,
    req: Request,
) -> Response {
    let (req, input): (_, RawBatchPayload) = match json_with_limit(req, MAX_RAW_BATCH_BODY).await {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let engine = match wiki(&state, &req) {
        Ok(engine) => engine,
        Err(resp) => return resp,
    };
    if input.files.is_empty() || input.files.len() > MAX_RAW_BATCH_FILES {
        return error(&req, StatusCode::BAD_REQUEST, "invalid_arguments");
    }
    let folder = space_relative(&input.folder_path);
    let mut created = Vec::new();
    let mut skipped = Vec::new();
    let mut failed = Vec::new();
    for file in &input.files {
        let Some(rel_path) = batch_rel_path(folder, &file.path) else {
            failed.push(serde_json::json!({ "path": file.path, "error": "invalid_path" }));
            continue;
        };
        if engine.config().sources_dir().join(&rel_path).exists() {
            skipped.push(format!("raw/{rel_path}"));
            continue;
        }
        match engine.write_source_file(&rel_path, &file.body) {
            Ok(info) => created.push(info.rel_path.replacen("sources/", "raw/", 1)),
            Err(err) => {
                failed.push(serde_json::json!({ "path": file.path, "error": err.to_string() }))
            }
        }
    }
    if !created.is_empty()
        && let Some(ref notify) = state.autoflow_notify
    {
        notify.notify_one();
    }
    let status = if created.is_empty() {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    (
        status,
        Json(serde_json::json!({
            "created": created,
            "skipped": skipped,
            "failed": failed,
        })),
    )
        .into_response()
}

pub(in crate::app::admin) async fn knowledge_raw_read(
    State(state): State<AppState>,
    Query(q): Query<RawPathQuery>,
    req: Request,
) -> Response {
    let engine = match wiki(&state, &req) {
        Ok(engine) => engine,
        Err(resp) => return resp,
    };
    let normalized = q
        .path
        .trim()
        .trim_start_matches('/')
        .replacen("raw/", "sources/", 1);
    match engine.get_document(&normalized, GetOptions::default()) {
        Ok(doc) => Json(serde_json::json!({
            "path": doc.info.rel_path.replacen("sources/", "raw/", 1),
            "docid": doc.info.docid,
            "body": doc.content,
            "sha256": sha256_hex(doc.content.as_bytes()),
        }))
        .into_response(),
        Err(err) => wiki_error(&req, err),
    }
}

#[derive(Deserialize)]
struct RawUpdatePayload {
    path: String,
    body: String,
    #[serde(default)]
    expected_sha256: Option<String>,
}

pub(in crate::app::admin) async fn knowledge_raw_update(
    State(state): State<AppState>,
    req: Request,
) -> Response {
    let (req, input): (_, RawUpdatePayload) = match json(req).await {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let engine = match wiki(&state, &req) {
        Ok(engine) => engine,
        Err(resp) => return resp,
    };
    let rel_path = input
        .path
        .trim()
        .trim_start_matches('/')
        .trim_start_matches("sources/")
        .trim_start_matches("raw/");
    let normalized = format!("sources/{rel_path}");
    if !verify_expected_sha256(&engine, &normalized, input.expected_sha256.as_deref()) {
        return error(&req, StatusCode::CONFLICT, "revision_conflict");
    }
    match engine.write_source_file(rel_path, &input.body) {
        Ok(info) => Json(serde_json::json!({
            "path": info.rel_path.replacen("sources/", "raw/", 1),
            "docid": info.docid,
            "body": input.body,
            "sha256": sha256_hex(input.body.as_bytes()),
        }))
        .into_response(),
        Err(err) => wiki_error(&req, err),
    }
}

#[derive(Deserialize)]
struct RawImportPayload {
    path: String,
    #[serde(default)]
    expected_sha256: Option<String>,
    name: String,
    provenance: String,
}

pub(in crate::app::admin) async fn knowledge_raw_import(
    State(state): State<AppState>,
    req: Request,
) -> Response {
    let (req, input): (_, RawImportPayload) = match json(req).await {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let engine = match wiki(&state, &req) {
        Ok(engine) => engine,
        Err(resp) => return resp,
    };
    let normalized = input
        .path
        .trim()
        .trim_start_matches('/')
        .replacen("raw/", "sources/", 1);
    if !verify_expected_sha256(&engine, &normalized, input.expected_sha256.as_deref()) {
        return error(&req, StatusCode::CONFLICT, "revision_conflict");
    }
    match engine.get_document(&normalized, GetOptions::default()) {
        Ok(doc) => {
            let mut body = serde_json::json!({
                "id": doc.info.rel_path,
                "docid": doc.info.docid,
                "version": 1,
                "name": input.name,
                "provenance": input.provenance,
                "sha256": sha256_hex(doc.content.as_bytes()),
                "size_bytes": doc.info.byte_size,
                "media_type": "text/markdown",
            });
            trigger_source_analysis(&state, &doc.info.rel_path, &mut body).await;
            (StatusCode::CREATED, Json(body)).into_response()
        }
        Err(err) => wiki_error(&req, err),
    }
}

#[derive(Deserialize)]
struct RawMovePayload {
    path: String,
    target_folder_path: String,
    file_name: String,
    #[serde(default)]
    expected_sha256: Option<String>,
}

pub(in crate::app::admin) async fn knowledge_raw_move(
    State(state): State<AppState>,
    req: Request,
) -> Response {
    let (req, input): (_, RawMovePayload) = match json(req).await {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let engine = match wiki(&state, &req) {
        Ok(engine) => engine,
        Err(resp) => return resp,
    };
    let from_rel = input
        .path
        .trim()
        .trim_start_matches('/')
        .trim_start_matches("sources/")
        .trim_start_matches("raw/");
    let normalized = format!("sources/{from_rel}");
    if !verify_expected_sha256(&engine, &normalized, input.expected_sha256.as_deref()) {
        return error(&req, StatusCode::CONFLICT, "revision_conflict");
    }
    let target_folder = space_relative(&input.target_folder_path);
    let to_rel = if target_folder.is_empty() {
        input.file_name.trim().to_string()
    } else {
        format!("{target_folder}/{}", input.file_name.trim())
    };
    match engine.move_file(CollectionScope::Sources, from_rel, &to_rel) {
        Ok(info) => Json(serde_json::json!({
            "path": info.rel_path.replacen("sources/", "raw/", 1),
            "docid": info.docid,
            "sha256": info.content_hash,
        }))
        .into_response(),
        Err(err) => wiki_error(&req, err),
    }
}

#[derive(Deserialize)]
struct WikiMovePayload {
    target_folder_path: String,
    file_name: String,
    #[serde(default)]
    expected_revision: Option<i64>,
}

pub(in crate::app::admin) async fn knowledge_page_move(
    State(state): State<AppState>,
    Path(id): Path<String>,
    req: Request,
) -> Response {
    let (req, input): (_, WikiMovePayload) = match json(req).await {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let engine = match wiki(&state, &req) {
        Ok(engine) => engine,
        Err(resp) => return resp,
    };
    if let Some(exp) = input.expected_revision
        && exp != 1
    {
        return error(&req, StatusCode::CONFLICT, "revision_conflict");
    }
    let from_rel = resolve_wiki_scoped_path(&engine, &id);
    let target_folder = input
        .target_folder_path
        .trim()
        .trim_matches('/')
        .trim_start_matches("wiki/");
    let to_rel = if target_folder.is_empty() {
        input.file_name.trim().to_string()
    } else {
        format!("{target_folder}/{}", input.file_name.trim())
    };
    match engine.move_file(CollectionScope::Wiki, &from_rel, &to_rel) {
        Ok(info) => {
            let _ = engine.rebuild_or_sync_index_md();
            Json(serde_json::json!({
                "path": info.rel_path,
                "docid": info.docid,
            }))
            .into_response()
        }
        Err(err) => wiki_error(&req, err),
    }
}
