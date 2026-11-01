//! Per-scope runtime state: one writer and one `watch` sender. No fact is cached here.

use super::types::MemoryContext;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, atomic::AtomicBool},
};
use tokio::sync::watch;

pub(super) struct ScopeEntry {
    pub(super) writer: tokio::sync::Mutex<()>,
    pub(super) context: watch::Sender<Arc<MemoryContext>>,
    /// Whether the last published snapshot came from disk, so `context` calls can skip the I/O.
    pub(super) loaded: AtomicBool,
}

#[derive(Default)]
pub(super) struct Registry(Mutex<HashMap<String, Arc<ScopeEntry>>>);

impl Registry {
    pub(super) fn entry(&self, key: &str) -> Arc<ScopeEntry> {
        let mut entries = self.0.lock().unwrap_or_else(|e| e.into_inner());
        entries
            .entry(key.to_string())
            .or_insert_with(|| {
                Arc::new(ScopeEntry {
                    writer: tokio::sync::Mutex::new(()),
                    context: watch::Sender::new(Arc::new(MemoryContext::default())),
                    loaded: AtomicBool::new(false),
                })
            })
            .clone()
    }

    pub(super) fn all(&self) -> Vec<(String, Arc<ScopeEntry>)> {
        let entries = self.0.lock().unwrap_or_else(|e| e.into_inner());
        entries
            .iter()
            .map(|(key, entry)| (key.clone(), entry.clone()))
            .collect()
    }
}
