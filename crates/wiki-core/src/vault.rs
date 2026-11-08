mod changes;
mod events;
mod lifecycle;
mod queries;
mod storage;

use crate::{Revision, VaultEvent};
use llm_wiki::WikiEngine;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use tokio::sync::broadcast;

pub struct Vault {
    root: PathBuf,
    engine: WikiEngine,
    write_lock: Mutex<()>,
    events: broadcast::Sender<VaultEvent>,
    lock_file: Mutex<Option<std::fs::File>>,
    writable: AtomicBool,
    self_writes: Mutex<HashMap<PathBuf, Option<Revision>>>,
}
