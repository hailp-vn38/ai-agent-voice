//! File-first Knowledge and Memory domains, sharing private vault/index machinery.
//! Neither Agent Job nor Admin may mutate canonical Markdown outside these cores.
mod sources;
pub(crate) mod vault;
use crate::database::knowledge_index as index;
use sources::SourceVault;
pub use sources::{Source, SourceInput};

use index::{Domain, SearchIndex};
pub use index::{Health, SearchHit};
use serde::Deserialize;
use std::{path::PathBuf, sync::Arc};
use uuid::Uuid;
use vault::MarkdownVault;
pub use vault::Record;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid_arguments")]
    InvalidInput,
    #[error("not_found")]
    NotFound,
    #[error("revision_conflict")]
    RevisionConflict,
    #[error("invalid_path")]
    InvalidPath,
    #[error("index_stale")]
    IndexStale,
    #[error("knowledge_unavailable")]
    Unavailable,
    #[error("storage_failure: {0}")]
    Io(#[from] std::io::Error),
    #[error("database_failure: {0}")]
    Sql(#[from] sqlx::Error),
}
impl Error {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidInput => "invalid_arguments",
            Self::NotFound => "not_found",
            Self::RevisionConflict => "revision_conflict",
            Self::InvalidPath => "invalid_path",
            Self::IndexStale => "index_stale",
            Self::Unavailable | Self::Io(_) | Self::Sql(_) => "knowledge_unavailable",
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WikiInput {
    pub id: Option<String>,
    pub title: String,
    pub body: String,
    pub expected_revision: Option<i64>,
    pub source_ref: Option<String>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FactInput {
    pub id: Option<String>,
    pub title: String,
    pub body: String,
    pub expected_revision: Option<i64>,
    pub source_ref: Option<String>,
    pub confirmed: bool,
}
#[derive(Clone, Debug, serde::Serialize)]
pub struct PageOverview {
    pub id: String,
    pub title: String,
    pub revision: i64,
    pub status: String,
    pub source_ref: Option<String>,
    pub excerpt: String,
}
pub struct KnowledgeCore {
    vault: MarkdownVault,
    index: SearchIndex,
    sources: SourceVault,
    drafts: MarkdownVault,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct FactOverview {
    pub id: String,
    pub title: String,
    pub revision: i64,
    pub status: String,
    pub confirmed: bool,
    pub excerpt: String,
}
pub struct MemoryCore {
    vault: MarkdownVault,
    index: SearchIndex,
}
pub struct Runtime {
    pub knowledge: KnowledgeCore,
    pub memory: MemoryCore,
}
impl Runtime {
    async fn check_root(root: &PathBuf, guard: &PathBuf) -> Result<(), Error> {
        if tokio::fs::symlink_metadata(guard).await.is_ok() {
            match tokio::fs::symlink_metadata(root).await {
                Ok(info) if info.is_dir() && !info.file_type().is_symlink() => {}
                _ => return Err(Error::Unavailable),
            }
        }
        Ok(())
    }
    async fn mark_root(guard: &PathBuf, data_root: &PathBuf) -> Result<(), Error> {
        match tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(guard)
            .await
        {
            Ok(file) => {
                file.sync_all().await?;
                vault::sync_dir(data_root.clone()).await?;
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(err) => return Err(err.into()),
        }
        Ok(())
    }
    pub async fn open(data_root: PathBuf) -> Result<Arc<Self>, Error> {
        let knowledge_root = data_root.join("knowledge");
        let memory_root = data_root.join("memory");
        let knowledge_guard = data_root.join(".knowledge_v3_initialized");
        let memory_guard = data_root.join(".memory_v3_initialized");
        // The guard is outside the vault: a missing whole mount must not be
        // mistaken for first bootstrap and allowed to prune indexed documents.
        Self::check_root(&knowledge_root, &knowledge_guard).await?;
        Self::check_root(&memory_root, &memory_guard).await?;
        let knowledge_vault = MarkdownVault::new(knowledge_root.clone(), "wiki").await?;
        let draft_vault = MarkdownVault::new(knowledge_root.clone(), "drafts").await?;
        let source_vault = SourceVault::open(&knowledge_root).await?;
        let memory_vault = MarkdownVault::new(memory_root.clone(), "facts").await?;
        let knowledge_index = SearchIndex::open(&knowledge_root, Domain::Knowledge).await?;
        let memory_index = SearchIndex::open(&memory_root, Domain::Memory).await?;
        let _startup_guard = vault::WRITE_LOCK.lock().await;
        // Startup reconciliation makes prior crash/power-loss repairs explicit.
        knowledge_index.reconcile(&knowledge_vault).await?;
        // Derived rows are not authoritative if immutable published evidence is
        // unavailable (e.g. Source volume detached while Wiki Markdown remained).
        for scanned in knowledge_vault.scan().await? {
            if scanned.record.status == "verified" {
                let verified = match scanned.record.source_ref.as_deref() {
                    Some(reference) => source_vault.verify(reference).await.is_ok(),
                    None => false,
                };
                if !verified {
                    knowledge_index.mark_stale().await?;
                    break;
                }
            }
        }
        memory_index.reconcile(&memory_vault).await?;
        let runtime = Arc::new(Self {
            knowledge: KnowledgeCore {
                vault: knowledge_vault,
                index: knowledge_index,
                sources: source_vault,
                drafts: draft_vault,
            },
            memory: MemoryCore {
                vault: memory_vault,
                index: memory_index,
            },
        });
        runtime.memory.regenerate_summary().await?;
        Self::mark_root(&knowledge_guard, &data_root).await?;
        Self::mark_root(&memory_guard, &data_root).await?;
        Ok(runtime)
    }
}
fn check_text(title: &str, body: &str) -> Result<(), Error> {
    if title.trim().is_empty()
        || title.len() > 512
        || body.len() > 256 * 1024
        || title.chars().any(|c| matches!(c, '\n' | '\r'))
    {
        return Err(Error::InvalidInput);
    }
    Ok(())
}
async fn old_record(vault: &MarkdownVault, id: &str) -> Result<Option<Record>, Error> {
    match vault.get(id).await {
        Ok(r) => Ok(Some(r)),
        Err(Error::NotFound) => Ok(None),
        Err(err) => Err(err),
    }
}
fn next_revision(old: Option<&Record>, expected: Option<i64>) -> Result<i64, Error> {
    let current = old.map_or(0, |r| r.revision);
    if current != expected.unwrap_or(-1) {
        return Err(Error::RevisionConflict);
    }
    current.checked_add(1).ok_or(Error::InvalidInput)
}

mod memory;
mod wiki;

#[cfg(test)]
mod tests;
