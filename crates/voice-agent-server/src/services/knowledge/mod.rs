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
    pub async fn open(data_root: PathBuf) -> Result<Arc<Self>, Error> {
        let knowledge_root = data_root.join("knowledge");
        let memory_root = data_root.join("memory");
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
                if !verified { knowledge_index.mark_stale().await?; break; }
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
    /// An accepted Wiki revision stays canonical while a replacement draft is
    /// edited in its own vault. Crash-left staged copies are ignored only when
    /// their content is exactly the already accepted new canonical revision.
    async fn staged_for(&self,canonical:&Record)->Result<Option<Record>,Error> {
        let Some(draft)=old_record(&self.drafts,&canonical.id).await? else {return Ok(None)};
        if draft.kind!="wiki" || draft.status!="draft" {return Err(Error::InvalidInput);}
        if draft.draft_base_revision==Some(canonical.revision) && canonical.status=="verified" {
            return Ok(Some(draft));
        }
        if canonical.status=="verified" &&
            draft.draft_base_revision.is_some_and(|base|base<canonical.revision) &&
            draft.title==canonical.title && draft.body==canonical.body {
            return Ok(None);
        }
        Err(Error::RevisionConflict)
    }
    async fn edit_record(&self,id:&str)->Result<Record,Error>{
        let canonical=self.vault.get(id).await?;
        if canonical.kind!="wiki"{return Err(Error::NotFound);}
        Ok(self.staged_for(&canonical).await?.unwrap_or(canonical))
    }
    fn range(mut rec:Record,from_line:usize,max_lines:usize)->Result<Record,Error>{
        if from_line==0 || !(1..=10000).contains(&max_lines){return Err(Error::InvalidInput);}
        if from_line!=1 || rec.body.lines().count()>max_lines {
            rec.body=rec.body.lines().skip(from_line-1).take(max_lines)
                .collect::<Vec<_>>().join("\n");
        }
        Ok(rec)
    }
    /// Admin-only list of canonical Wiki drafts and published pages. Job tools
    /// only receive verified documents through search/read_verified.
    pub async fn admin_list(&self, limit: usize) -> Result<Vec<PageOverview>, Error> {
        if !(1..=200).contains(&limit) {
            return Err(Error::InvalidInput);
        }
        let _guard = self.vault.writer.lock().await;
        let mut pages = Vec::new();
        for scanned in self.vault.scan().await? {
            let record = self.staged_for(&scanned.record).await?.unwrap_or(scanned.record);
            if record.kind != "wiki" {
                return Err(Error::InvalidInput);
            }
            pages.push(PageOverview {
                id: record.id,
                title: record.title,
                revision: record.revision,
                status: record.status,
                source_ref: record.source_ref,
                excerpt: record.body.chars().take(600).collect(),
            });
        }
        pages.truncate(limit);
        Ok(pages)
    }
    pub async fn import_source(&self, input: SourceInput) -> Result<Source, Error> {
        self.sources.import_markdown(input).await
    }
    pub async fn list_sources(&self) -> Result<Vec<Source>, Error> {
        self.sources.list().await
    }
    pub async fn read_source(&self, id: &str) -> Result<String, Error> {
        self.sources.read_text(id).await
    }
    /// Publish a new or replacement draft after independently verifying Source
    /// evidence and the expected draft revision (optimistic CAS).
    pub async fn publish(&self,id:&str,expected_revision:i64,source_ref:&str)->Result<Record,Error>{
        let _guard=self.vault.writer.lock().await;
        let canonical=self.vault.get(id).await?;
        if canonical.kind!="wiki" {return Err(Error::NotFound);}
        let staged=self.staged_for(&canonical).await?;
        let (draft,revision)=if let Some(draft)=staged.as_ref(){
            // The staging revision is a separate CAS space. The canonical Wiki
            // revision advances exactly once when the revision is accepted.
            next_revision(Some(draft),Some(expected_revision))?;
            if canonical.status!="verified" {return Err(Error::RevisionConflict);}
            (draft.clone(),canonical.revision.checked_add(1).ok_or(Error::InvalidInput)?)
        } else {
            if canonical.status!="draft" {return Err(Error::InvalidInput);}
            let revision=next_revision(Some(&canonical),Some(expected_revision))?;
            (canonical.clone(),revision)
        };
        self.sources.verify(source_ref).await?;
        let mut rec=draft;
        rec.revision=revision;
        rec.status="verified".into();
        rec.draft_base_revision=None;
        rec.source_ref=Some(source_ref.to_owned());
        self.index.mark_stale().await?;
        self.vault.commit(&rec,Some(&canonical)).await?;
        self.index.reconcile(&self.vault).await?;
        // A crash between canonical commit and cleanup is detected by staged_for.
        if staged.is_some() {
            if let Err(error)=self.drafts.remove_draft(id).await {
                tracing::warn!(wiki_id=%id,%error,"accepted Wiki revision has pending draft cleanup");
            }
        }
        Ok(rec)
    }
    /// Updating published Wiki creates a *staged* draft; the accepted revision
    /// and Agent-readable FTS projection remain unchanged until publication.
    pub async fn put_draft(&self,input:WikiInput)->Result<Record,Error>{
        check_text(&input.title,&input.body)?;
        let id=input.id.unwrap_or_else(||format!("wiki_{}",Uuid::new_v4().simple()));
        let _guard=self.vault.writer.lock().await;
        let previous=old_record(&self.vault,&id).await?;
        if previous.as_ref().is_some_and(|r|r.kind!="wiki") {return Err(Error::InvalidInput);}
        if let Some(canonical)=previous.as_ref().filter(|r|r.status=="verified"){
            let staged=self.staged_for(canonical).await?;
            let revision=if let Some(draft)=staged.as_ref(){
                next_revision(Some(draft),input.expected_revision)?
            } else {
                let next=next_revision(Some(canonical),input.expected_revision)?;
                // Remove a crash-left staged copy of an already published revision
                // before starting a new draft against this canonical base.
                if old_record(&self.drafts,&id).await?.is_some() {
                    self.drafts.remove_draft(&id).await?;
                }
                next
            };
            let rec=Record {
                id,kind:"wiki".into(),title:input.title,body:input.body,
                revision,status:"draft".into(),scope:None,confirmed:false,
                source_ref:input.source_ref,
                draft_base_revision:Some(canonical.revision)
            };
            self.drafts.commit(&rec,staged.as_ref()).await?;
            return Ok(rec);
        }
        let revision=next_revision(previous.as_ref(),input.expected_revision)?;
        let rec=Record {
            id,kind:"wiki".into(),title:input.title,body:input.body,
            revision,status:"draft".into(),scope:None,confirmed:false,
            source_ref:input.source_ref,draft_base_revision:None
        };
        self.index.mark_stale().await?;
        self.vault.commit(&rec,previous.as_ref()).await?;
        self.index.reconcile(&self.vault).await?;
        Ok(rec)
    }
    /// Line range is for browsing only; the Admin editor calls read_full.
    pub async fn read(&self,id:&str,from_line:usize,max_lines:usize)->Result<Record,Error>{
        let _guard=self.vault.writer.lock().await;
        let record=self.edit_record(id).await?;
        Self::range(record,from_line,max_lines)
    }
    pub async fn read_full(&self,id:&str)->Result<Record,Error>{
        let _guard=self.vault.writer.lock().await;
        self.edit_record(id).await
    }
    pub async fn search(&self, q: &str, limit: i64) -> Result<Vec<SearchHit>, Error> {
        let _guard = self.vault.writer.lock().await;
        let mut hits=self.index.search(q,limit).await?;
        for hit in &mut hits {
            let page=self.vault.get(&hit.id).await?;
            if page.status!="verified" || page.kind!="wiki" ||
                !self.verify_reference(page.source_ref.as_deref()).await {
                self.index.mark_stale().await?;
                return Err(Error::IndexStale);
            }
            hit.source_ref=page.source_ref;
        }
        Ok(hits)
    }
    async fn verify_reference(&self,source_ref:Option<&str>)->bool {
        match source_ref {
            Some(reference)=>self.sources.verify(reference).await.is_ok(),
            None=>false
        }
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
        // Never read staged edits through an Agent operation.
        let _guard=self.vault.writer.lock().await;
        let page=self.vault.get(id).await?;
        if page.kind!="wiki" || page.status!="verified"{return Err(Error::NotFound);}
        if !self.verify_reference(page.source_ref.as_deref()).await {
            self.index.mark_stale().await?;
            return Err(Error::IndexStale);
        }
        Self::range(page,from_line,max_lines)
    }
    pub async fn health(&self) -> Result<Health, Error> {
        self.index.health().await
    }
    pub async fn rebuild(&self) -> Result<usize, Error> {
        let _guard = self.vault.writer.lock().await;
        for scanned in self.vault.scan().await? {
            if scanned.record.status == "verified" &&
                !self.verify_reference(scanned.record.source_ref.as_deref()).await {
                self.index.mark_stale().await?;
                return Err(Error::IndexStale);
            }
        }
        self.index.rebuild(&self.vault).await
    }
}
impl MemoryCore {
    /// Admin-only overview, intentionally independent of FTS and containing
    /// unpublished/forgotten facts for management. Job tools cannot call it.
    pub async fn admin_list(&self, limit: usize) -> Result<Vec<FactOverview>, Error> {
        if !(1..=200).contains(&limit) {
            return Err(Error::InvalidInput);
        }
        let _guard = self.vault.writer.lock().await;
        let mut items = Vec::new();
        for item in self.vault.scan().await? {
            let record = item.record;
            if record.kind != "memory" || record.scope.as_deref() != Some("owner") {
                return Err(Error::InvalidInput);
            }
            items.push(FactOverview {
                id: record.id,
                title: record.title,
                revision: record.revision,
                status: record.status.clone(),
                confirmed: record.confirmed,
                excerpt: if record.status == "forgotten" {
                    String::new()
                } else {
                    record.body.chars().take(600).collect()
                },
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
            draft_base_revision: None,
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
        vault::sync_dir(parent.to_path_buf()).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
