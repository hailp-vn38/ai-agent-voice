//! Owner-scoped canonical Memory facts, summary and forgetting.
use super::*;
use std::path::Path;
use tokio::{fs,io::AsyncWriteExt};

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
    pub(super) fn generate(docs: &[vault::Scanned]) -> Result<String, Error> {
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
    pub(super) async fn regenerate_summary(&self) -> Result<(), Error> {
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
