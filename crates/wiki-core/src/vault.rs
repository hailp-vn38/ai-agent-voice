mod changes;
mod events;
mod lifecycle;
mod queries;
mod storage;

use crate::{Revision, VaultEvent};
use llm_wiki::WikiEngine;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Mutex, RwLock, RwLockReadGuard};
use tokio::sync::broadcast;

pub struct Vault {
    root: PathBuf,
    engine: RwLock<WikiEngine>,
    /// Private Search Projection of a reader; dropped once it takes the lock.
    reader_index: Mutex<Option<tempfile::TempDir>>,
    write_lock: Mutex<()>,
    events: broadcast::Sender<VaultEvent>,
    lock_file: Mutex<Option<std::fs::File>>,
    writable: AtomicBool,
    self_writes: Mutex<HashMap<PathBuf, Option<Revision>>>,
}

impl Vault {
    pub(crate) fn root_path(&self) -> &std::path::Path {
        &self.root
    }

    fn engine(&self) -> Result<RwLockReadGuard<'_, WikiEngine>, crate::VaultError> {
        self.engine
            .read()
            .map_err(|_| crate::VaultError::Io("vault engine poisoned".into()))
    }
}
