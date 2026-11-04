//! Phát hiện file Markdown chép thẳng vào `sources/` để Auto-Flow ingest.
//!
//! `WikiEngine::sync_index()` đọc và hash mọi file của vault, giải lại toàn bộ link và giữ khóa
//! database suốt thời gian chạy. Vòng này chỉ `stat` các file `.md` trong `sources/` và gọi
//! `sync_index()` khi danh sách đường dẫn, mtime hoặc kích thước thay đổi.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use llm_wiki::WikiEngine;
use tokio::sync::Notify;

/// Chu kỳ kiểm tra metadata của `sources/`.
const SOURCES_POLL_INTERVAL: Duration = Duration::from_secs(30);

/// Metadata của từng file `.md` dưới `sources/`, khóa theo đường dẫn tương đối.
pub(crate) type SourcesSnapshot = BTreeMap<PathBuf, (Option<SystemTime>, u64)>;

/// Chụp metadata các file `.md` (bỏ qua file và thư mục ẩn) mà không đọc nội dung.
pub(crate) fn snapshot_sources(root: &Path) -> SourcesSnapshot {
    let mut out = SourcesSnapshot::new();
    let mut stack = vec![root.to_path_buf()];
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
                continue;
            }
            let is_md = path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("md"));
            if !file_type.is_file() || !is_md {
                continue;
            }
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            let rel = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
            out.insert(rel, (meta.modified().ok(), meta.len()));
        }
    }
    out
}

/// Chạy nền: so snapshot mỗi chu kỳ, chỉ sync index khi `sources/` đổi và báo Auto-Flow khi
/// index thật sự có nguồn mới, sửa hoặc xóa. Sync lỗi thì giữ snapshot cũ để thử lại.
pub(crate) fn spawn_sources_disk_sync(engine: Arc<WikiEngine>, autoflow_notify: Arc<Notify>) {
    let root = engine.config().sources_dir();
    tokio::spawn(async move {
        // Lúc khởi động index đã được sync, nên snapshot đầu tiên là mốc so sánh.
        let mut previous = {
            let root = root.clone();
            tokio::task::spawn_blocking(move || snapshot_sources(&root))
                .await
                .unwrap_or_default()
        };
        let mut ticker = tokio::time::interval(SOURCES_POLL_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        ticker.tick().await;
        loop {
            ticker.tick().await;
            let root = root.clone();
            let engine = engine.clone();
            let prev = std::mem::take(&mut previous);
            let outcome = tokio::task::spawn_blocking(move || {
                let current = snapshot_sources(&root);
                if current == prev {
                    return (current, Ok(false));
                }
                match engine.sync_index() {
                    Ok(stats) => (
                        current,
                        Ok(stats.indexed_new + stats.updated + stats.removed > 0),
                    ),
                    Err(err) => (prev, Err(err)),
                }
            })
            .await;
            match outcome {
                Ok((snapshot, result)) => {
                    previous = snapshot;
                    match result {
                        Ok(true) => autoflow_notify.notify_one(),
                        Ok(false) => {}
                        Err(err) => tracing::warn!(error = %err, "wiki sources disk sync failed"),
                    }
                }
                Err(err) => tracing::warn!(error = %err, "wiki sources disk sync task panicked"),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("vela-sources-watch-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn snapshot_tracks_nested_markdown_and_ignores_hidden_and_other_files() {
        let root = temp_root("nested");
        std::fs::create_dir_all(root.join("handbook/guide")).unwrap();
        std::fs::create_dir_all(root.join(".obsidian")).unwrap();
        std::fs::write(root.join("handbook/intro.md"), "# Intro").unwrap();
        std::fs::write(root.join("handbook/guide/setup.MD"), "# Setup").unwrap();
        std::fs::write(root.join("handbook/logo.png"), "png").unwrap();
        std::fs::write(root.join("handbook/.draft.md"), "hidden").unwrap();
        std::fs::write(root.join(".obsidian/notes.md"), "hidden").unwrap();

        let snapshot = snapshot_sources(&root);
        let paths: Vec<_> = snapshot.keys().cloned().collect();
        assert_eq!(
            paths,
            vec![
                PathBuf::from("handbook/guide/setup.MD"),
                PathBuf::from("handbook/intro.md"),
            ]
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn snapshot_changes_when_markdown_is_added_edited_or_removed() {
        let root = temp_root("changes");
        std::fs::write(root.join("a.md"), "one").unwrap();
        let base = snapshot_sources(&root);
        assert_eq!(base, snapshot_sources(&root));

        std::fs::write(root.join("logo.png"), "png").unwrap();
        assert_eq!(
            base,
            snapshot_sources(&root),
            "non-Markdown files are ignored"
        );

        std::fs::write(root.join("a.md"), "one more").unwrap();
        let edited = snapshot_sources(&root);
        assert_ne!(base, edited);

        std::fs::write(root.join("b.md"), "two").unwrap();
        let added = snapshot_sources(&root);
        assert_ne!(edited, added);

        std::fs::remove_file(root.join("b.md")).unwrap();
        assert_eq!(edited, snapshot_sources(&root));
        let _ = std::fs::remove_dir_all(&root);
    }
}
