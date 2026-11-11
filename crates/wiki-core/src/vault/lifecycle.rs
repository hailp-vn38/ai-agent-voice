//! Vault initialization and exclusive writer lock ownership.

use super::Vault;
use crate::{VaultConfig, VaultError, VaultEvent};
use llm_wiki::{WikiConfig, WikiEngine};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, RwLock};
use tokio::sync::broadcast;

fn try_lock_vault(root: &Path) -> Result<Option<std::fs::File>, VaultError> {
    let path = root.join(".wiki-desktop.lock");
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(path)
        .map_err(|error| VaultError::Io(error.to_string()))?;
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if result == 0 {
            Ok(Some(file))
        } else {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::WouldBlock {
                Ok(None)
            } else {
                Err(VaultError::Io(error.to_string()))
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = file;
        Err(VaultError::Io(
            "advisory vault locking requires a Unix platform".into(),
        ))
    }
}

/// Files the engine would create on open; a reader must find them in place.
const SCAFFOLD: [&str; 8] = [
    "SCHEMA.md",
    "sources/assets",
    "wiki/sources",
    "wiki/entities",
    "wiki/concepts",
    "wiki/syntheses",
    "wiki/index.md",
    "wiki/log.md",
];

/// Opens the engine. The lock holder owns the shared Search Projection; a
/// reader indexes into a private SQLite file so it never writes to the vault.
pub(super) fn open_engine(
    root: &Path,
    writable: bool,
) -> Result<(WikiEngine, Option<tempfile::TempDir>), VaultError> {
    if writable {
        return Ok((WikiEngine::open_with_config(WikiConfig::new(root))?, None));
    }
    if let Some(missing) = SCAFFOLD.iter().find(|path| !root.join(path).exists()) {
        return Err(VaultError::Io(format!(
            "vault is locked by another process and is not initialized ({missing} is missing)"
        )));
    }
    let index_dir = tempfile::Builder::new()
        .prefix("wiki-vault-reader-")
        .tempdir()
        .map_err(|error| VaultError::Io(error.to_string()))?;
    let mut config = WikiConfig::new(root);
    config.db_rel_path = index_dir.path().join("index.sqlite");
    config.init_obsidian_config = false;
    Ok((WikiEngine::open_with_config(config)?, Some(index_dir)))
}

impl Vault {
    pub fn open(config: VaultConfig) -> Result<Self, VaultError> {
        std::fs::create_dir_all(&config.root).map_err(|error| VaultError::Io(error.to_string()))?;
        let root = config
            .root
            .canonicalize()
            .map_err(|error| VaultError::Io(error.to_string()))?;
        for folder in ["sources", "wiki"] {
            let scoped_root = root.join(folder);
            if scoped_root.exists()
                && !scoped_root
                    .canonicalize()
                    .map_err(|error| VaultError::Io(error.to_string()))?
                    .starts_with(&root)
            {
                return Err(VaultError::InvalidPath(scoped_root.display().to_string()));
            }
        }
        let lock_file = try_lock_vault(&root)?;
        let writable = lock_file.is_some();
        let (engine, reader_index) = open_engine(&root, writable)?;
        let (events, _) = broadcast::channel(64);
        Ok(Self {
            root,
            engine: RwLock::new(engine),
            reader_index: Mutex::new(reader_index),
            write_lock: Mutex::new(()),
            events,
            lock_file: Mutex::new(lock_file),
            writable: AtomicBool::new(writable),
            self_writes: Mutex::new(HashMap::new()),
        })
    }

    pub fn is_writable(&self) -> bool {
        self.writable.load(Ordering::Acquire)
    }

    pub fn try_acquire_write(&self) -> Result<bool, VaultError> {
        if self.is_writable() {
            return Ok(true);
        }
        let mut lock_file = self
            .lock_file
            .lock()
            .map_err(|_| VaultError::Io("vault lock handle poisoned".into()))?;
        if self.is_writable() {
            return Ok(true);
        }
        if let Some(file) = try_lock_vault(&self.root)? {
            let (engine, _) = open_engine(&self.root, true)?;
            let _guard = self
                .write_lock
                .lock()
                .map_err(|_| VaultError::Io("vault write lock poisoned".into()))?;
            *self
                .engine
                .write()
                .map_err(|_| VaultError::Io("vault engine poisoned".into()))? = engine;
            if let Ok(mut reader_index) = self.reader_index.lock() {
                reader_index.take();
            }
            *lock_file = Some(file);
            self.writable.store(true, Ordering::Release);
            let _ = self.events.send(VaultEvent::Lock { writable: true });
            return Ok(true);
        }
        Ok(false)
    }
}
