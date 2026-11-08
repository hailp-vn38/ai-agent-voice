//! Vault initialization and exclusive writer lock ownership.

use super::Vault;
use crate::{VaultConfig, VaultError, VaultEvent};
use llm_wiki::{WikiConfig, WikiEngine};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
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
        let engine = WikiEngine::open_with_config(WikiConfig::new(&root))?;
        let (events, _) = broadcast::channel(64);
        Ok(Self {
            root,
            engine,
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
            *lock_file = Some(file);
            self.writable.store(true, Ordering::Release);
            let _ = self.events.send(VaultEvent::Lock { writable: true });
            return Ok(true);
        }
        Ok(false)
    }
}
