//! SSE feed that tells Admin clients when the Knowledge vault changed.
//!
//! Each connection compares file metadata under `wiki/` and `sources/` and the index database.
//! It only `stat`s files, so it never takes the engine's database lock or runs lint, and it sends
//! an event only when something changed. Events are hints: clients still read state over REST.

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use axum::extract::{Request, State};
use axum::http::{HeaderValue, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};

use crate::app::AppState;

/// How often a connection compares the vault metadata.
const CHECK_INTERVAL: Duration = Duration::from_secs(1);
/// A write touches several files, so wait for it to settle before telling clients.
const SETTLE_DELAY: Duration = Duration::from_millis(300);
/// Above this many changed paths the event carries `truncated: true` and clients reload everything.
const MAX_PATHS: usize = 200;
/// Key prefix for index database files, which are never reported as changed paths.
const INDEX_KEY: &str = "#index/";

/// Metadata of every watched file, keyed by its Admin path (`wiki/...` or `raw/...`).
type Snapshot = BTreeMap<String, (Option<SystemTime>, u64)>;

struct WatchedRoots {
    /// Vault folders and the Admin path prefix their files use.
    folders: Vec<(PathBuf, &'static str)>,
    /// SQLite files whose changes mean the index (graph, counts, embeddings) moved.
    index_files: Vec<PathBuf>,
}

fn snapshot(roots: &WatchedRoots) -> Snapshot {
    let mut out = Snapshot::new();
    for (root, prefix) in &roots.folders {
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                if entry.file_name().to_string_lossy().starts_with('.') {
                    continue;
                }
                let Ok(file_type) = entry.file_type() else {
                    continue;
                };
                let path = entry.path();
                if file_type.is_dir() {
                    stack.push(path);
                } else if file_type.is_file()
                    && let Ok(meta) = entry.metadata()
                {
                    out.insert(
                        admin_path(root, &path, prefix),
                        (meta.modified().ok(), meta.len()),
                    );
                }
            }
        }
    }
    for file in &roots.index_files {
        if let Ok(meta) = std::fs::metadata(file) {
            let name = file.file_name().unwrap_or_default().to_string_lossy();
            out.insert(
                format!("{INDEX_KEY}{name}"),
                (meta.modified().ok(), meta.len()),
            );
        }
    }
    out
}

fn admin_path(root: &Path, path: &Path, prefix: &str) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    let parts: Vec<_> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect();
    format!("{prefix}/{}", parts.join("/"))
}

/// File paths that were added, removed or modified between two snapshots.
fn changed_paths(before: &Snapshot, after: &Snapshot) -> (Vec<String>, bool) {
    let mut paths: Vec<String> = before
        .iter()
        .filter(|(key, value)| after.get(*key) != Some(value))
        .map(|(key, _)| key)
        .chain(after.keys().filter(|key| !before.contains_key(*key)))
        .filter(|key| !key.starts_with(INDEX_KEY))
        .cloned()
        .collect();
    paths.sort();
    let truncated = paths.len() > MAX_PATHS;
    paths.truncate(MAX_PATHS);
    (paths, truncated)
}

async fn take_snapshot(roots: &Arc<WatchedRoots>) -> Option<Snapshot> {
    let roots = roots.clone();
    tokio::task::spawn_blocking(move || snapshot(&roots))
        .await
        .ok()
}

/// `GET /api/admin/knowledge/events`: `ready` once, then `knowledge_changed` per vault change.
pub(in crate::app::admin) async fn knowledge_events(
    State(state): State<AppState>,
    req: Request,
) -> Response {
    let engine = match super::wiki(&state, &req) {
        Ok(engine) => engine,
        Err(resp) => return resp,
    };
    let config = engine.config();
    let db = config.db_path();
    let mut wal = db.clone().into_os_string();
    wal.push("-wal");
    let roots = Arc::new(WatchedRoots {
        folders: vec![(config.wiki_dir(), "wiki"), (config.sources_dir(), "raw")],
        index_files: vec![db, PathBuf::from(wal)],
    });
    let stopping = state.lifecycle.stopping().clone();
    let sync_events = state
        .wiki_sync_service
        .get()
        .map(|service| service.subscribe());

    let messages = futures_util::stream::unfold(
        (None::<Snapshot>, roots, stopping, sync_events),
        |(previous, roots, stopping, mut sync_events)| async move {
            let Some(mut previous) = previous else {
                // Take the baseline before `ready`, so any change after it produces an event.
                let baseline = take_snapshot(&roots).await.unwrap_or_default();
                return Some((
                    Ok::<Event, Infallible>(Event::default().event("ready").data("{}")),
                    (Some(baseline), roots, stopping, sync_events),
                ));
            };
            loop {
                tokio::select! {
                    biased;
                    _ = stopping.cancelled() => return None,
                    event = async { match sync_events.as_mut() { Some(events) => events.recv().await, None => std::future::pending::<Result<_, _>>().await } } => {
                        let status = match event {
                            Ok(status) => status,
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                                // A fresh GET gives the current state after a lagged subscriber reconnects.
                                continue;
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                                sync_events = None;
                                continue;
                            }
                        };
                        let data = serde_json::to_string(&status).unwrap_or_else(|_| "{}".into());
                        return Some((Ok(Event::default().event("sync_status").data(data)), (Some(previous), roots, stopping, sync_events)));
                    }
                    _ = tokio::time::sleep(CHECK_INTERVAL) => {}
                }
                let Some(current) = take_snapshot(&roots).await else {
                    continue;
                };
                if current == previous {
                    continue;
                }
                tokio::select! {
                    biased;
                    _ = stopping.cancelled() => return None,
                    _ = tokio::time::sleep(SETTLE_DELAY) => {}
                }
                let settled = take_snapshot(&roots).await.unwrap_or(current);
                if settled == previous {
                    continue;
                }
                let (paths, truncated) = changed_paths(&previous, &settled);
                previous = settled;
                let data = serde_json::json!({ "paths": paths, "truncated": truncated });
                return Some((
                    Ok(Event::default()
                        .event("knowledge_changed")
                        .data(data.to_string())),
                    (Some(previous), roots, stopping, sync_events),
                ));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(len: u64) -> (Option<SystemTime>, u64) {
        (Some(SystemTime::UNIX_EPOCH), len)
    }

    #[test]
    fn changed_paths_reports_added_removed_and_modified_files_but_not_index_files() {
        let before = Snapshot::from([
            ("wiki/kept.md".into(), entry(1)),
            ("wiki/edited.md".into(), entry(1)),
            ("raw/removed.md".into(), entry(1)),
            (format!("{INDEX_KEY}wiki.db"), entry(1)),
        ]);
        let after = Snapshot::from([
            ("wiki/kept.md".into(), entry(1)),
            ("wiki/edited.md".into(), entry(2)),
            ("raw/added.md".into(), entry(1)),
            (format!("{INDEX_KEY}wiki.db"), entry(2)),
        ]);
        let (paths, truncated) = changed_paths(&before, &after);
        assert_eq!(paths, ["raw/added.md", "raw/removed.md", "wiki/edited.md"]);
        assert!(!truncated);
    }
}
