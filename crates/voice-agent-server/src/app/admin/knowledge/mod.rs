//! Admin HTTP handlers for Knowledge and Memory routes backed by `llm_wiki::WikiEngine`
//! and `orchestrator_flow_rs::TaskManager`.

mod graph;
mod memory;
mod pages;
pub(super) mod processing;
mod raw;
mod sources;
#[cfg(test)]
mod tests;

pub(super) use graph::{knowledge_graph, knowledge_health, knowledge_rebuild};
pub(super) use memory::{
    memory_create, memory_forget, memory_health, memory_read, memory_rebuild, memory_recall,
    memory_summary, memory_update,
};
pub(super) use pages::{
    PageLines, SearchQuery, knowledge_create, knowledge_publish, knowledge_read, knowledge_search,
    knowledge_update,
};
pub(super) use raw::{
    knowledge_folder_create, knowledge_page_move, knowledge_raw_create, knowledge_raw_import,
    knowledge_raw_move, knowledge_raw_read, knowledge_raw_update, knowledge_tree,
};
pub(super) use sources::{
    knowledge_chart_read, knowledge_import_source, knowledge_read_source, knowledge_sources,
};

use super::*;
use llm_wiki::{GetOptions, WikiEngine, WikiError};
use sha2::{Digest, Sha256};
use std::sync::Arc;

#[allow(clippy::result_large_err)]
pub(super) fn wiki(state: &AppState, req: &Request) -> Result<Arc<WikiEngine>, Response> {
    state.wiki_engine.as_ref().cloned().ok_or_else(|| {
        error(
            req,
            StatusCode::SERVICE_UNAVAILABLE,
            "knowledge_unavailable",
        )
    })
}

pub(super) fn wiki_error(req: &Request, err: WikiError) -> Response {
    match err {
        WikiError::DocumentNotFound { .. } => error(req, StatusCode::NOT_FOUND, "not_found"),
        WikiError::PathTraversal(_) | WikiError::InvalidFileName(_) => {
            error(req, StatusCode::BAD_REQUEST, "invalid_path")
        }
        WikiError::InvalidMetadataFilter(_) | WikiError::InvalidToolArguments { .. } => {
            error(req, StatusCode::BAD_REQUEST, "invalid_arguments")
        }
        _ => error(
            req,
            StatusCode::SERVICE_UNAVAILABLE,
            "knowledge_unavailable",
        ),
    }
}

pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub(super) fn resolve_wiki_scoped_path(engine: &WikiEngine, id: &str) -> String {
    if let Ok(existing) = engine.get_document(id, GetOptions::default()) {
        existing.info.scoped_path
    } else {
        id.trim_start_matches("wiki/").to_string()
    }
}

pub(super) async fn trigger_source_analysis(state: &AppState, rel_path: &str, body: &mut Value) {
    let disabled = state
        .admin_task_control
        .lock()
        .expect("admin_task_control mutex poisoned")
        .background_enabled_override
        == Some(false);
    if disabled {
        return;
    }
    if let Some(ref tm) = state.task_manager
        && let Ok(task) =
            crate::services::task_flow::submit_wiki_source_analysis(tm, rel_path).await
    {
        body["analysis"] = serde_json::json!({
            "job_id": task.id,
            "session_id": task.session_id,
            "turn_id": task.latest_turn_id,
            "replayed": false,
        });
    }
    if let Some(ref notify) = state.autoflow_notify {
        notify.notify_one();
    }
}

pub(super) fn format_wiki_markdown(title: &str, body: &str, source_ref: Option<&str>) -> String {
    if body.trim_start().starts_with("---") {
        return body.to_string();
    }
    let mut out = format!("---\ntitle: \"{}\"\n", title.replace('"', "\\\""));
    if let Some(src) = source_ref.filter(|s| !s.trim().is_empty()) {
        out.push_str(&format!("sources:\n  - \"[[{src}]]\"\n"));
    }
    out.push_str("---\n\n");
    if !body.trim_start().starts_with("# ") {
        out.push_str(&format!("# {title}\n\n"));
    }
    out.push_str(body);
    out
}
