//! Everything derived from disk: vault state, `MemoryContext`, `MEMORY.md`, `recall`, `inspect`.

use super::{
    Inner,
    fact_file::{self, FactFile, read_profile},
    fit_summary::{SummaryRow, fit_summary, joined_len, render_line, sort_rows},
    index,
    normalize::compare_key,
    scopes::ScopeEntry,
    tombstone::{Tombstones, fingerprint},
    types::{
        FactStatus, InspectFilter, MemoryContext, MemoryError, MemoryOverview, MemoryScope,
        RecallQuery, RecallResult, ScopeTarget,
    },
    vault::{self, DevicePaths},
};
use std::{cmp::Reverse, collections::HashMap, io, sync::Arc, sync::atomic::Ordering};

const MAX_PROFILE_LINE_BYTES: usize = 1_000;
const RECALL_OVERFETCH: usize = 20;

/// Runs filesystem work off the async workers. Losing the task counts as storage being unavailable.
pub(super) async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> io::Result<T> + Send + 'static,
) -> Result<T, MemoryError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|_| MemoryError::Unavailable)?
        .map_err(unavailable)
}

pub(super) fn unavailable(error: io::Error) -> MemoryError {
    tracing::warn!(event = "memory_storage_error", kind = ?error.kind());
    MemoryError::Unavailable
}

/// One Device vault as read from disk. `residual` holds tombstoned files that a crash left behind.
#[derive(Debug)]
pub(super) struct VaultState {
    pub(super) live: Vec<FactFile>,
    pub(super) residual: Vec<FactFile>,
    pub(super) quarantined: Vec<String>,
    pub(super) tombstones: Tombstones,
}

pub(super) fn read_state(
    paths: &DevicePaths,
    key: &[u8; 32],
    device_dir: &str,
) -> io::Result<VaultState> {
    let tombstones = Tombstones::read(&paths.tombstones)?;
    let scan = fact_file::scan(&paths.facts)?;
    let (residual, live) = scan
        .facts
        .into_iter()
        .partition(|file| tombstones.contains(&fingerprint(key, device_dir, &file.fact.body)));
    Ok(VaultState {
        live,
        residual,
        quarantined: scan.quarantined,
        tombstones,
    })
}

/// The only facts `recall` and `context` may ever show.
fn is_visible(file: &FactFile) -> bool {
    file.fact.status == FactStatus::Confirmed && file.fact.superseded_by.is_none()
}

pub(super) fn visible_facts(state: &VaultState) -> Vec<&super::types::MemoryFact> {
    state
        .live
        .iter()
        .filter(|file| is_visible(file))
        .map(|file| &file.fact)
        .collect()
}

fn summary_rows(state: &VaultState) -> Vec<SummaryRow> {
    visible_facts(state)
        .into_iter()
        .map(|fact| SummaryRow {
            id: fact.id.as_str().to_string(),
            title: fact.title.clone(),
            hook: fact.hook.clone(),
            kind: fact.kind,
            pinned: fact.pinned,
            updated_at: fact.updated_at,
        })
        .collect()
}

/// The profile as one JSON line of at most `max_bytes`, or nothing if it is empty or cannot fit.
fn profile_line(body: &str, max_bytes: usize) -> Option<String> {
    if body.is_empty() {
        return None;
    }
    let chars: Vec<char> = body.chars().collect();
    let render = |count: usize| {
        let text: String = chars[..count].iter().collect();
        render_line(&serde_json::json!({ "profile": text }))
    };
    if render(chars.len()).len() <= max_bytes {
        return Some(render(chars.len()));
    }
    let (mut fits, mut too_long) = (0, chars.len());
    while too_long - fits > 1 {
        let middle = fits + (too_long - fits) / 2;
        if render(middle).len() <= max_bytes {
            fits = middle;
        } else {
            too_long = middle;
        }
    }
    (fits > 0).then(|| render(fits))
}

/// A broken `profile.md` costs the profile line, not the whole Memory Summary.
fn context_from(inner: &Inner, rows: &[SummaryRow]) -> MemoryContext {
    let budget = inner.voice_summary_max_bytes;
    let profile = match read_profile(&inner.vault.profile_path()) {
        Ok(file) => profile_line(&file.profile.body, budget.min(MAX_PROFILE_LINE_BYTES)),
        Err(error) => {
            tracing::warn!(event = "memory_profile_unreadable", kind = ?error.kind());
            None
        }
    };
    let mut lines: Vec<String> = profile.into_iter().collect();
    let used = joined_len(&lines) + usize::from(!lines.is_empty());
    let fitted = fit_summary(rows, budget.saturating_sub(used));
    lines.extend(fitted.lines);
    MemoryContext {
        rows: lines,
        hidden: fitted.hidden,
    }
}

fn render_summary_file(rows: &[SummaryRow]) -> String {
    let mut sorted: Vec<&SummaryRow> = rows.iter().collect();
    sort_rows(&mut sorted);
    let mut out =
        String::from("# Memory\n\n<!-- Generated from facts/. Edits here are overwritten. -->\n\n");
    for row in sorted {
        out.push_str(&format!(
            "- **{}** ({}): {}\n",
            row.title,
            row.kind.as_str(),
            row.hook
        ));
    }
    out
}

/// What publishing needs and `MEMORY.md` needs, from one consistent read.
pub(super) struct Derived {
    pub(super) context: MemoryContext,
    pub(super) summary_file: String,
}

pub(super) fn derive(inner: &Inner, state: &VaultState) -> Derived {
    let rows = summary_rows(state);
    Derived {
        context: context_from(inner, &rows),
        summary_file: render_summary_file(&rows),
    }
}

pub(super) fn publish(entry: &ScopeEntry, context: MemoryContext) {
    entry.context.send_replace(Arc::new(context));
    entry.loaded.store(true, Ordering::Release);
}

/// Fail closed: an unreadable vault must not leave a forgotten fact in the last snapshot. This
/// snapshot did not come from disk, so the next `context` call reads again.
pub(super) fn publish_degraded(entry: &ScopeEntry) {
    tracing::warn!(event = "memory_context_degraded");
    entry
        .context
        .send_replace(Arc::new(MemoryContext::default()));
    entry.loaded.store(false, Ordering::Release);
}

/// Reads the scope from disk and publishes it. The caller holds the writer.
pub(super) async fn publish_from_disk(
    inner: &Arc<Inner>,
    scope_key: &str,
    entry: &ScopeEntry,
) -> Result<Derived, MemoryError> {
    let inner_for_read = inner.clone();
    let key = scope_key.to_string();
    let read = blocking(move || {
        let state = read_state(
            &inner_for_read.vault.device(&key),
            &inner_for_read.key,
            &key,
        )?;
        Ok(derive(&inner_for_read, &state))
    })
    .await;
    match read {
        Ok(derived) => {
            publish(entry, derived.context.clone());
            Ok(derived)
        }
        Err(error) => {
            publish_degraded(entry);
            Err(error)
        }
    }
}

/// The deployment scope has no facts of its own: its context is only the profile line.
pub(super) async fn publish_deployment(
    inner: &Arc<Inner>,
    entry: &ScopeEntry,
) -> Result<(), MemoryError> {
    let inner = inner.clone();
    match blocking(move || Ok(context_from(&inner, &[]))).await {
        Ok(context) => {
            publish(entry, context);
            Ok(())
        }
        Err(error) => {
            publish_degraded(entry);
            Err(error)
        }
    }
}

pub(super) async fn recall(
    inner: &Arc<Inner>,
    scope: &MemoryScope,
    query: RecallQuery,
) -> Result<RecallResult, MemoryError> {
    let ScopeTarget::Device { dir } = &scope.0 else {
        return Err(MemoryError::ScopeDenied);
    };
    let paths = inner.vault.device(dir);
    let candidates = query.limit.saturating_add(RECALL_OVERFETCH);

    // Searching must not create a vault for a Device that has none.
    if !paths.dir.exists() {
        return Ok(RecallResult {
            items: Vec::new(),
            hidden: 0,
        });
    }
    if !paths.index.exists() {
        rebuild_index(inner, dir, &paths).await?;
    }
    let ids = match index::search(&paths.index, &query.text, candidates).await {
        Ok(ids) => ids,
        Err(error) => {
            tracing::warn!(event = "memory_index_rebuilt", kind = %sqlx_kind(&error));
            rebuild_index(inner, dir, &paths).await?;
            index::search(&paths.index, &query.text, candidates)
                .await
                .map_err(|error| {
                    tracing::warn!(event = "memory_index_unavailable", kind = %sqlx_kind(&error));
                    MemoryError::Unavailable
                })?
        }
    };

    let state = read_device(inner, dir).await?;
    let by_id: HashMap<&str, &FactFile> = state
        .live
        .iter()
        .filter(|file| is_visible(file))
        .map(|file| (file.fact.id.as_str(), file))
        .collect();
    let matches: Vec<&FactFile> = ids
        .iter()
        .filter_map(|id| by_id.get(id.as_str()).copied())
        .collect();
    let hidden = matches.len().saturating_sub(query.limit);
    Ok(RecallResult {
        items: matches
            .into_iter()
            .take(query.limit)
            .map(|file| file.fact.clone())
            .collect(),
        hidden,
    })
}

/// Never logs the error's text: a SQLite message can quote the statement or the data.
fn sqlx_kind(error: &sqlx::Error) -> &'static str {
    match error {
        sqlx::Error::Database(_) => "database",
        sqlx::Error::Io(_) => "io",
        _ => "other",
    }
}

async fn read_device(inner: &Arc<Inner>, dir: &str) -> Result<VaultState, MemoryError> {
    let inner = inner.clone();
    let dir = dir.to_string();
    blocking(move || read_state(&inner.vault.device(&dir), &inner.key, &dir)).await
}

/// Holds the writer, so a rebuild cannot miss a fact being written right now.
async fn rebuild_index(
    inner: &Arc<Inner>,
    dir: &str,
    paths: &DevicePaths,
) -> Result<(), MemoryError> {
    let entry = inner.registry.entry(dir);
    let _writer = entry.writer.lock().await;
    let state = read_device(inner, dir).await?;
    index::rebuild(&paths.index, &visible_facts(&state))
        .await
        .map_err(|error| {
            tracing::warn!(event = "memory_index_unavailable", kind = %sqlx_kind(&error));
            MemoryError::Unavailable
        })
}

pub(super) async fn inspect(
    inner: &Arc<Inner>,
    scope: &MemoryScope,
    filter: InspectFilter,
) -> Result<MemoryOverview, MemoryError> {
    let dir = match &scope.0 {
        ScopeTarget::Deployment => {
            let path = inner.vault.profile_path();
            let file = blocking(move || read_profile(&path)).await?;
            return Ok(MemoryOverview {
                facts: Vec::new(),
                quarantined: 0,
                quarantined_files: Vec::new(),
                profile: Some(file.profile),
            });
        }
        ScopeTarget::Device { dir } => dir,
    };
    let state = read_device(inner, dir).await?;
    let mut facts: Vec<_> = state.live.into_iter().map(|file| file.fact).collect();
    if let Some(id) = &filter.id {
        facts.retain(|fact| &fact.id == id);
        if facts.is_empty() {
            return Err(MemoryError::NotFound);
        }
    }
    facts.sort_by_key(|fact| (!fact.pinned, Reverse(fact.updated_at), fact.id.clone()));
    Ok(MemoryOverview {
        facts,
        quarantined: state.quarantined.len(),
        quarantined_files: state.quarantined,
        profile: None,
    })
}

pub(super) fn same_body(left: &str, right: &str) -> bool {
    compare_key(&fact_file::canonical_text(left)) == compare_key(&fact_file::canonical_text(right))
}

pub(super) fn write_summary_file(paths: &DevicePaths, content: &str) -> io::Result<()> {
    vault::write_atomic(&paths.summary, content.as_bytes())
}
