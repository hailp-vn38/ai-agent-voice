//! Vault layout and crash-safe file primitives. No other module builds a vault path.

use super::types::{MemoryScope, ScopeTarget};
use std::{
    fs, io,
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

const TEMP_SUFFIX: &str = ".tmp";

/// Paths of one Device vault. `dir` may not exist yet; it appears on the first write.
#[derive(Clone, Debug)]
pub(super) struct DevicePaths {
    pub(super) dir: PathBuf,
    pub(super) facts: PathBuf,
    pub(super) summary: PathBuf,
    pub(super) tombstones: PathBuf,
    pub(super) index: PathBuf,
}

#[derive(Clone, Debug)]
pub(super) struct Vault {
    root: PathBuf,
}

impl Vault {
    pub(super) fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
        }
    }

    pub(super) fn devices_dir(&self) -> PathBuf {
        self.root.join("devices")
    }

    pub(super) fn deployment_dir(&self) -> PathBuf {
        self.root.join("_deployment")
    }

    pub(super) fn profile_path(&self) -> PathBuf {
        self.deployment_dir().join("profile.md")
    }

    pub(super) fn tombstone_key_path(&self) -> PathBuf {
        self.deployment_dir().join("tombstone.key")
    }

    pub(super) fn device(&self, dir: &str) -> DevicePaths {
        let dir = self.devices_dir().join(dir);
        DevicePaths {
            facts: dir.join("facts"),
            summary: dir.join("MEMORY.md"),
            tombstones: dir.join("tombstones.json"),
            index: dir.join(".index.sqlite"),
            dir,
        }
    }

    pub(super) fn scope_key(scope: &MemoryScope) -> String {
        match &scope.0 {
            ScopeTarget::Device { dir } => dir.clone(),
            // `_` never starts a Device directory name, which is hex or `web-assistant`.
            ScopeTarget::Deployment => "_deployment".to_string(),
        }
    }
}

/// Creates a directory and makes its entry durable.
pub(super) fn ensure_dir(dir: &Path) -> io::Result<()> {
    let created: Vec<&Path> = dir
        .ancestors()
        .take_while(|path| !path.as_os_str().is_empty() && !path.exists())
        .collect();
    if created.is_empty() {
        return Ok(());
    }
    fs::create_dir_all(dir)?;
    for path in created {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            sync_dir(parent)?;
        }
    }
    Ok(())
}

fn sync_dir(dir: &Path) -> io::Result<()> {
    fs::File::open(dir)?.sync_all()
}

/// A hidden name next to `path`, so `facts/*.md` scans never see it and `rename` stays atomic.
pub(super) fn temp_path(path: &Path) -> io::Result<PathBuf> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
    Ok(path.with_file_name(format!(
        ".{name}.{}{TEMP_SUFFIX}",
        uuid::Uuid::new_v4().simple()
    )))
}

/// Temp file in the same directory, `fsync`, `rename`, `fsync` of the directory.
pub(super) fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    write_atomic_with_mode(path, bytes, 0o644)
}

pub(super) fn write_atomic_with_mode(path: &Path, bytes: &[u8], mode: u32) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
    ensure_dir(parent)?;
    let temp = temp_path(path)?;
    let written = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temp, path)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&temp);
    }
    written?;
    sync_dir(parent)
}

/// Deleting something already gone is success, so cleanup can be retried.
pub(super) fn remove_durable(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    }
    match path.parent() {
        Some(parent) => sync_dir(parent),
        None => Ok(()),
    }
}

/// Removes leftovers of interrupted writes from `dir` and returns how many there were.
pub(super) fn remove_temp_files(dir: &Path) -> io::Result<usize> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error),
    };
    let mut removed = 0;
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        // `-journal` is what SQLite leaves beside an index that was being built.
        if name.starts_with('.') && (name.ends_with(TEMP_SUFFIX) || name.ends_with(".tmp-journal"))
        {
            remove_durable(&entry.path())?;
            removed += 1;
        }
    }
    Ok(removed)
}

/// Reads the 32-byte tombstone key, creating it with mode `0600` on first open. A key of the wrong
/// length is an error, never replaced: regenerating would silently void every existing tombstone.
pub(super) fn load_or_create_key(path: &Path) -> io::Result<[u8; 32]> {
    match fs::read(path) {
        Ok(bytes) => {
            let key = <[u8; 32]>::try_from(bytes.as_slice())
                .map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
            Ok(key)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let mut key = [0_u8; 32];
            ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut key)
                .map_err(|_| io::Error::other("system random source failed"))?;
            write_atomic_with_mode(path, &key, 0o600)?;
            Ok(key)
        }
        Err(error) => Err(error),
    }
}
