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
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::{fs, io::AsyncWriteExt};
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
pub struct KnowledgeCore {
    vault: MarkdownVault,
    index: SearchIndex,
    sources: SourceVault,
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
    pub async fn open(data_root: PathBuf) -> Result<Arc<Self>, Error> {
        let knowledge_root = data_root.join("knowledge");
        let memory_root = data_root.join("memory");
        let knowledge_vault = MarkdownVault::new(knowledge_root.clone(), "wiki").await?;
        let source_vault = SourceVault::open(&knowledge_root).await?;
        let memory_vault = MarkdownVault::new(memory_root.clone(), "facts").await?;
        let knowledge_index = SearchIndex::open(&knowledge_root, Domain::Knowledge).await?;
        let memory_index = SearchIndex::open(&memory_root, Domain::Memory).await?;
        let _startup_guard = vault::WRITE_LOCK.lock().await;
        // Startup reconciliation makes prior crash/power-loss repairs explicit.
        knowledge_index.reconcile(&knowledge_vault).await?;
        memory_index.reconcile(&memory_vault).await?;
        let runtime = Arc::new(Self {
            knowledge: KnowledgeCore {
                vault: knowledge_vault,
                index: knowledge_index,
                sources: source_vault,
            },
            memory: MemoryCore {
                vault: memory_vault,
                index: memory_index,
            },
        });
        runtime.memory.regenerate_summary().await?;
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
impl KnowledgeCore {
    pub async fn import_source(&self, input: SourceInput) -> Result<Source, Error> {
        self.sources.import_markdown(input).await
    }
    pub async fn list_sources(&self) -> Result<Vec<Source>, Error> {
        self.sources.list().await
    }
    pub async fn read_source(&self, id: &str) -> Result<String, Error> {
        self.sources.read_text(id).await
    }
    /// Only a verified immutable Source allows a draft to become published.
    pub async fn publish(
        &self,
        id: &str,
        expected_revision: i64,
        source_ref: &str,
    ) -> Result<Record, Error> {
        let _guard = self.vault.writer.lock().await;
        let prior = self.vault.get(id).await?;
        if prior.kind != "wiki" || prior.status != "draft" {
            return Err(Error::InvalidInput);
        }
        let revision = next_revision(Some(&prior), Some(expected_revision))?;
        self.sources.verify(source_ref).await?;
        let mut rec = prior.clone();
        rec.revision = revision;
        rec.status = "verified".into();
        rec.source_ref = Some(source_ref.to_owned());
        self.index.mark_stale().await?;
        self.vault.commit(&rec, Some(&prior)).await?;
        self.index.reconcile(&self.vault).await?;
        Ok(rec)
    }
    /// Admin drafts only. Verified publication requires a validated immutable
    /// Source/version and is intentionally not smuggled into this endpoint.
    pub async fn put_draft(&self, input: WikiInput) -> Result<Record, Error> {
        check_text(&input.title, &input.body)?;
        let id = input
            .id
            .unwrap_or_else(|| format!("wiki_{}", Uuid::new_v4().simple()));
        let _guard = self.vault.writer.lock().await;
        let previous = old_record(&self.vault, &id).await?;
        if previous
            .as_ref()
            .is_some_and(|r| r.kind != "wiki" || r.status == "verified")
        {
            return Err(Error::InvalidInput);
        }
        let revision = next_revision(previous.as_ref(), input.expected_revision)?;
        let rec = Record {
            id,
            kind: "wiki".into(),
            title: input.title,
            body: input.body,
            revision,
            status: "draft".into(),
            scope: None,
            confirmed: false,
            source_ref: input.source_ref,
        };
        self.index.mark_stale().await?;
        self.vault.commit(&rec, previous.as_ref()).await?;
        self.index.reconcile(&self.vault).await?;
        Ok(rec)
    }
    pub async fn read(
        &self,
        id: &str,
        from_line: usize,
        max_lines: usize,
    ) -> Result<Record, Error> {
        if from_line == 0 || !(1..=10000).contains(&max_lines) {
            return Err(Error::InvalidInput);
        }
        let _guard = self.vault.writer.lock().await;
        let mut rec = self.vault.get(id).await?;
        if rec.kind != "wiki" {
            return Err(Error::NotFound);
        }
        if from_line != 1 || rec.body.lines().count() > max_lines {
            rec.body = rec
                .body
                .lines()
                .skip(from_line - 1)
                .take(max_lines)
                .collect::<Vec<_>>()
                .join("\n");
        }
        Ok(rec)
    }
    pub async fn search(&self, q: &str, limit: i64) -> Result<Vec<SearchHit>, Error> {
        let _guard = self.vault.writer.lock().await;
        self.index.search(q, limit).await
    }
    pub async fn admin_search(&self, q: &str, limit: i64) -> Result<Vec<SearchHit>, Error> {
        let _guard = self.vault.writer.lock().await;
        self.index.admin_search(q, limit).await
    }
    pub async fn read_verified(
        &self,
        id: &str,
        from_line: usize,
        max_lines: usize,
    ) -> Result<Record, Error> {
        let page = self.read(id, from_line, max_lines).await?;
        if page.status != "verified" {
            return Err(Error::NotFound);
        }
        Ok(page)
    }
    pub async fn health(&self) -> Result<Health, Error> {
        self.index.health().await
    }
    pub async fn rebuild(&self) -> Result<usize, Error> {
        let _guard = self.vault.writer.lock().await;
        self.index.rebuild(&self.vault).await
    }
}
impl MemoryCore {
    /// Admin-only overview, intentionally independent of FTS and containing
    /// unpublished/forgotten facts for management. Job tools cannot call it.
    pub async fn admin_list(&self, limit: usize) -> Result<Vec<FactOverview>, Error> {
        if !(1..=200).contains(&limit) { return Err(Error::InvalidInput); }
        let _guard = self.vault.writer.lock().await;
        let mut items = Vec::new();
        for item in self.vault.scan().await? {
            let record=item.record;
            if record.kind!="memory" || record.scope.as_deref()!=Some("owner") {
                return Err(Error::InvalidInput);
            }
            items.push(FactOverview {
                id:record.id,
                title:record.title,
                revision:record.revision,
                status:record.status.clone(),
                confirmed:record.confirmed,
                excerpt:if record.status=="forgotten" {String::new()}
                    else {record.body.chars().take(600).collect()},
            });
        }
        items.truncate(limit);
        Ok(items)
    }
    pub async fn put_fact(&self, input: FactInput) -> Result<Record, Error> {
        check_text(&input.title, &input.body)?;
        if input.source_ref.as_deref().unwrap_or("").trim().is_empty() {
            return Err(Error::InvalidInput);
        }
        let id = input
            .id
            .unwrap_or_else(|| format!("mem_{}", Uuid::new_v4().simple()));
        let _guard = self.vault.writer.lock().await;
        let previous = old_record(&self.vault, &id).await?;
        if previous.as_ref().is_some_and(|r| {
            r.kind != "memory" || r.scope.as_deref() != Some("owner") || r.status == "forgotten"
        }) {
            return Err(Error::InvalidInput);
        }
        let revision = next_revision(previous.as_ref(), input.expected_revision)?;
        let rec = Record {
            id,
            kind: "memory".into(),
            title: input.title,
            body: input.body,
            revision,
            status: if input.confirmed {
                "active".into()
            } else {
                "needs_review".into()
            },
            scope: Some("owner".into()),
            confirmed: input.confirmed,
            source_ref: input.source_ref,
        };
        self.index.mark_stale().await?;
        self.vault.commit(&rec, previous.as_ref()).await?;
        // Publish the derived summary BEFORE reporting success, even if indexing
        // later fails. A stale summary must never be served by summary().
        self.regenerate_summary().await?;
        self.index.reconcile(&self.vault).await?;
        Ok(rec)
    }
    pub async fn forget(&self, id: &str, expected_revision: i64) -> Result<Record, Error> {
        let _guard = self.vault.writer.lock().await;
        let previous = self.vault.get(id).await?;
        if previous.kind != "memory"
            || previous.scope.as_deref() != Some("owner")
            || previous.status == "forgotten"
        {
            return Err(Error::InvalidInput);
        }
        let revision = next_revision(Some(&previous), Some(expected_revision))?;
        let mut rec = previous.clone();
        rec.revision = revision;
        rec.status = "forgotten".into();
        rec.confirmed = false;
        self.index.mark_stale().await?;
        self.vault.commit(&rec, Some(&previous)).await?;
        self.regenerate_summary().await?;
        self.index.reconcile(&self.vault).await?;
        Ok(rec)
    }
    fn generate(docs: &[vault::Scanned]) -> Result<String, Error> {
        let mut text = String::from("# Memory\n\n");
        for doc in docs {
            let fact = &doc.record;
            if fact.scope.as_deref() != Some("owner") || fact.kind != "memory" {
                return Err(Error::InvalidInput);
            }
            if fact.status != "active" || !fact.confirmed {
                continue;
            }
            let line = format!(
                "- {}: {}\n",
                fact.title,
                fact.body.split_whitespace().collect::<Vec<_>>().join(" ")
            );
            if text.len() + line.len() > 4096 {
                break;
            }
            text.push_str(&line);
        }
        Ok(text)
    }
    /// Must be called under writer lock, except during Runtime::open before sharing.
    async fn regenerate_summary(&self) -> Result<(), Error> {
        let docs = self.vault.scan().await?;
        let text = Self::generate(&docs)?;
        let path = self.vault.root().join("MEMORY.md");
        atomic_summary_write(&path, &text).await?;
        Ok(())
    }
    pub async fn summary(&self) -> Result<String, Error> {
        let _guard = self.vault.writer.lock().await;
        // A stale generated file after a crash is never treated as current.
        let docs = self.vault.scan().await?;
        let expected = Self::generate(&docs)?;
        let actual = fs::read_to_string(self.vault.root().join("MEMORY.md"))
            .await
            .map_err(|_| Error::IndexStale)?;
        if actual != expected {
            return Err(Error::IndexStale);
        }
        Ok(actual)
    }
    pub async fn read(&self, id: &str) -> Result<Record, Error> {
        let _guard = self.vault.writer.lock().await;
        let rec = self.vault.get(id).await?;
        if rec.kind != "memory" || rec.scope.as_deref() != Some("owner") {
            return Err(Error::NotFound);
        }
        Ok(rec)
    }
    pub async fn recall(&self, q: &str, limit: i64) -> Result<Vec<SearchHit>, Error> {
        let _guard = self.vault.writer.lock().await;
        self.index.search(q, limit).await
    }
    pub async fn health(&self) -> Result<Health, Error> {
        self.index.health().await
    }
    pub async fn rebuild(&self) -> Result<usize, Error> {
        let _guard = self.vault.writer.lock().await;
        let count = self.index.rebuild(&self.vault).await?;
        self.regenerate_summary().await?;
        Ok(count)
    }
}
async fn atomic_summary_write(path: &Path, content: &str) -> Result<(), Error> {
    let temp = path.with_extension(format!("{}.tmp", Uuid::new_v4()));
    let result = async {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .await?;
        f.write_all(content.as_bytes()).await?;
        f.sync_all().await?;
        drop(f);
        fs::rename(&temp, path).await
    }
    .await;
    if result.is_err() {
        let _ = fs::remove_file(&temp).await;
    }
    result?;
    if let Some(parent) = path.parent() {
        std::fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn expected_revision_is_fail_closed() {
        assert!(matches!(
            next_revision(None, None),
            Err(Error::RevisionConflict)
        ));
        assert_eq!(next_revision(None, Some(0)).unwrap(), 1);
        assert!(matches!(
            next_revision(None, Some(1)),
            Err(Error::RevisionConflict)
        ));
    }

    #[tokio::test]
    async fn isolated_indexes_and_canonical_revision_cas() {
        let root = std::env::temp_dir().join(format!("knowledge_v3_{}", Uuid::new_v4()));
        let core = Runtime::open(root.clone()).await.unwrap();
        let wiki = core
            .knowledge
            .put_draft(WikiInput {
                id: None,
                title: "Agent Session".into(),
                body: "A durable agent session".into(),
                expected_revision: Some(0),
                source_ref: Some("src_1:1".into()),
            })
            .await
            .unwrap();
        assert_eq!(wiki.revision, 1);
        assert_eq!(
            core.knowledge
                .admin_search("durable", 10)
                .await
                .unwrap()
                .len(),
            1
        );
        assert!(
            core.knowledge
                .search("durable", 10)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(matches!(
            core.knowledge.read_verified(&wiki.id, 1, 20).await,
            Err(Error::NotFound)
        ));
        assert!(matches!(
            core.knowledge
                .put_draft(WikiInput {
                    id: Some(wiki.id.clone()),
                    title: "conflict".into(),
                    body: "bad update".into(),
                    expected_revision: Some(0),
                    source_ref: None
                })
                .await,
            Err(Error::RevisionConflict)
        ));

        let evidence = core
            .knowledge
            .import_source(SourceInput {
                name: "evidence.md".into(),
                content: "# Original durable fact".into(),
                provenance: "admin:test".into(),
            })
            .await
            .unwrap();
        let published = core
            .knowledge
            .publish(&wiki.id, wiki.revision, &format!("{}:1", evidence.id))
            .await
            .unwrap();
        assert_eq!(published.status, "verified");
        assert_eq!(core.knowledge.search("durable", 10).await.unwrap().len(), 1);
        let fact = core
            .memory
            .put_fact(FactInput {
                id: None,
                title: "Language".into(),
                body: "The user prefers Vietnamese".into(),
                expected_revision: Some(0),
                source_ref: Some("admin".into()),
                confirmed: true,
            })
            .await
            .unwrap();
        assert_eq!(core.memory.recall("Vietnamese", 5).await.unwrap().len(), 1);
        assert_eq!(
            core.knowledge.search("Vietnamese", 5).await.unwrap().len(),
            0
        );
        assert!(core.memory.summary().await.unwrap().contains("Vietnamese"));
        core.memory.forget(&fact.id, fact.revision).await.unwrap();
        assert!(matches!(
            core.memory
                .put_fact(FactInput {
                    id: Some(fact.id.clone()),
                    title: "Resurrected".into(),
                    body: "Must not revive forgotten memory".into(),
                    expected_revision: Some(fact.revision + 1),
                    source_ref: Some("admin".into()),
                    confirmed: true,
                })
                .await,
            Err(Error::InvalidInput)
        ));
        assert!(
            core.memory
                .recall("Vietnamese", 5)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(!core.memory.summary().await.unwrap().contains("Vietnamese"));
        assert_eq!(core.knowledge.health().await.unwrap().document_count, 1);
        assert_eq!(core.memory.health().await.unwrap().document_count, 1);
        drop(core);
        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn admin_listing_includes_unconfirmed_but_job_recall_does_not() {
        let root = std::env::temp_dir().join(format!("memory_v3_list_{}",Uuid::new_v4()));
        let core = Runtime::open(root.clone()).await.unwrap();
        let draft=core.memory.put_fact(FactInput {
            id:None,title:"Pending preference".into(),
            body:"Private unconfirmed phrase".into(),
            expected_revision:Some(0),source_ref:Some("admin:test".into()),confirmed:false
        }).await.unwrap();
        let items=core.memory.admin_list(25).await.unwrap();
        assert_eq!(items.len(),1);
        assert_eq!(items[0].status,"needs_review");
        assert_eq!(items[0].id,draft.id);
        assert!(core.memory.recall("unconfirmed",10).await.unwrap().is_empty());
        core.memory.forget(&draft.id,draft.revision).await.unwrap();
        let items=core.memory.admin_list(25).await.unwrap();
        assert_eq!(items.len(),1);
        assert_eq!(items[0].status,"forgotten");
        assert!(items[0].excerpt.is_empty());
        drop(core);
        let _=tokio::fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn forced_rebuild_repairs_missing_fts_row_without_rewriting_markdown() {
        let root = std::env::temp_dir().join(format!("knowledge_v3_{}", Uuid::new_v4()));
        let core = Runtime::open(root.clone()).await.unwrap();
        let page = core
            .knowledge
            .put_draft(WikiInput {
                id: None,
                title: "Recovery".into(),
                body: "uniqueindexprobe".into(),
                expected_revision: Some(0),
                source_ref: None,
            })
            .await
            .unwrap();
        let canonical =
            tokio::fs::read(root.join("knowledge/wiki").join(format!("{}.md", page.id)))
                .await
                .unwrap();
        let uri = format!(
            "sqlite://{}",
            root.join("knowledge/knowledge_index.db").display()
        );
        let index = sqlx::SqlitePool::connect(&uri).await.unwrap();
        sqlx::query("DELETE FROM kb_fts WHERE stable_id=?")
            .bind(&page.id)
            .execute(&index)
            .await
            .unwrap();
        assert!(
            core.knowledge
                .admin_search("uniqueindexprobe", 10)
                .await
                .unwrap()
                .is_empty()
        );
        core.knowledge.rebuild().await.unwrap();
        assert_eq!(
            core.knowledge
                .admin_search("uniqueindexprobe", 10)
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            canonical,
            tokio::fs::read(root.join("knowledge/wiki").join(format!("{}.md", page.id)))
                .await
                .unwrap()
        );
        drop(index);
        drop(core);
        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn missing_vault_root_fails_closed_without_mass_delete() {
        let root = std::env::temp_dir().join(format!("knowledge_v3_{}", Uuid::new_v4()));
        let core = Runtime::open(root.clone()).await.unwrap();
        let page = core
            .knowledge
            .put_draft(WikiInput {
                id: None,
                title: "Record".into(),
                body: "nonempty".into(),
                expected_revision: Some(0),
                source_ref: None,
            })
            .await
            .unwrap();
        tokio::fs::rename(
            root.join("knowledge/wiki"),
            root.join("knowledge/wiki-offline"),
        )
        .await
        .unwrap();
        assert!(core.knowledge.rebuild().await.is_err());
        assert!(matches!(
            core.knowledge.search("nonempty", 10).await,
            Err(Error::IndexStale)
        ));
        assert_eq!(core.knowledge.health().await.unwrap().document_count, 1);
        tokio::fs::rename(
            root.join("knowledge/wiki-offline"),
            root.join("knowledge/wiki"),
        )
        .await
        .unwrap();
        core.knowledge.rebuild().await.unwrap();
        assert_eq!(
            core.knowledge.read(&page.id, 1, 10).await.unwrap().body,
            "nonempty"
        );
        drop(core);
        let _ = tokio::fs::remove_dir_all(root).await;
    }
    #[test]
    fn memory_summary_only_confirmed_active() {
        let rec = Record {
            id: "mem_1".into(),
            kind: "memory".into(),
            title: "Pref".into(),
            body: "Vietnamese".into(),
            revision: 1,
            status: "forgotten".into(),
            scope: Some("owner".into()),
            confirmed: false,
            source_ref: Some("admin".into()),
        };
        let scanned = vault::Scanned {
            record: rec,
            relative_path: String::new(),
            content_sha256: String::new(),
            mtime_ns: 0,
            size_bytes: 0,
        };
        assert_eq!(MemoryCore::generate(&[scanned]).unwrap(), "# Memory\n\n");
    }
}
