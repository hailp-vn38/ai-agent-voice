//! Vault notifications and suppression of filesystem events from our own writes.

use super::Vault;
use crate::{DocPath, ReindexStats, Revision, VaultError, VaultEvent};
use llm_wiki::SyncStats;
use std::path::PathBuf;
use tokio::sync::broadcast;

impl Vault {
    pub fn subscribe(&self) -> broadcast::Receiver<VaultEvent> {
        self.events.subscribe()
    }

    pub fn notice_external(
        &self,
        paths: impl IntoIterator<Item = DocPath>,
    ) -> Result<bool, VaultError> {
        // A reader still refreshes its private index and tells the UI; only the
        // lock holder may rewrite `wiki/index.md` inside the vault.
        let writable = self.is_writable();
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| VaultError::Io("vault write lock poisoned".into()))?;
        let paths: Vec<PathBuf> = paths
            .into_iter()
            .map(|path| path.vault_relative())
            .collect();
        if paths.is_empty() {
            return Ok(false);
        }
        let mut external = Vec::new();
        let mut self_writes = self
            .self_writes
            .lock()
            .map_err(|_| VaultError::Io("vault self-write set poisoned".into()))?;
        for relative in paths {
            let current = match std::fs::read(self.root.join(&relative)) {
                Ok(content) => Some(Revision::of(&content)),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(VaultError::Io(error.to_string())),
            };
            match self_writes.get(&relative) {
                Some(written) if written == &current => continue,
                Some(_) => {
                    self_writes.remove(&relative);
                }
                None => {}
            }
            external.push(relative);
        }
        drop(self_writes);
        if external.is_empty() {
            return Ok(false);
        }
        self.engine()?.sync_index()?;
        if writable {
            self.engine()?.rebuild_or_sync_index_md()?;
            let _ = self.record_self_writes(&["wiki/index.md".into()]);
        }
        let external: Vec<String> = external
            .iter()
            .map(|path| path.display().to_string())
            .collect();
        let _ = self.events.send(VaultEvent::External { paths: external });
        Ok(true)
    }

    pub(super) fn record_self_writes(&self, paths: &[String]) -> Result<(), VaultError> {
        let mut self_writes = self
            .self_writes
            .lock()
            .map_err(|_| VaultError::Io("vault self-write set poisoned".into()))?;
        for path in paths {
            let relative = PathBuf::from(path);
            let revision = match std::fs::read(self.root.join(&relative)) {
                Ok(content) => Some(Revision::of(&content)),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(VaultError::Io(error.to_string())),
            };
            self_writes.insert(relative, revision);
        }
        Ok(())
    }

    pub(super) fn publish_reindexed(&self, stats: SyncStats) {
        let Ok(engine) = self.engine() else {
            return;
        };
        if let Ok(status) = engine.status() {
            let _ = self.events.send(VaultEvent::Reindexed {
                stats: ReindexStats {
                    raw_documents: status.sources_count,
                    wiki_pages: status.wiki_count,
                    total_chunks: stats.total_chunks,
                },
            });
        }
    }
}
