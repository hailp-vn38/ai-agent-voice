//! Scoped filesystem writes and snapshots used to roll back failed changes.

use super::Vault;
use crate::{DocPath, Scope, VaultError};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

struct FileSnapshot {
    relative: PathBuf,
    content: Option<Vec<u8>>,
}

struct DirectorySnapshot {
    relative: PathBuf,
    existed: bool,
}

pub(super) struct SnapshotSet {
    files: Vec<FileSnapshot>,
    directories: Vec<DirectorySnapshot>,
}

static TEMP_FILE_ID: AtomicU64 = AtomicU64::new(0);

fn write_atomic(target: &Path, content: &[u8]) -> Result<(), VaultError> {
    let parent = target
        .parent()
        .ok_or_else(|| VaultError::InvalidPath(target.display().to_string()))?;
    let file_name = target
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| VaultError::InvalidPath(target.display().to_string()))?;
    let id = TEMP_FILE_ID.fetch_add(1, Ordering::Relaxed);
    let temp = parent.join(format!(".{file_name}.wiki-tmp-{}-{id}", std::process::id()));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|error| VaultError::Io(error.to_string()))?;
    if let Err(error) = file.write_all(content).and_then(|()| file.sync_all()) {
        let _ = std::fs::remove_file(&temp);
        return Err(VaultError::Io(error.to_string()));
    }
    drop(file);
    std::fs::rename(&temp, target).map_err(|error| {
        let _ = std::fs::remove_file(&temp);
        VaultError::Io(error.to_string())
    })
}

impl Vault {
    pub(super) fn validate_target(&self, path: &DocPath) -> Result<(), VaultError> {
        let scope_root = self.root.join(match path.scope() {
            Scope::Raw => "sources",
            Scope::Wiki => "wiki",
            Scope::All => unreachable!(),
        });
        self.validate_destination_parent(&self.root.join(path.vault_relative()), path.scope())?;
        if let Ok(canonical) = self.root.join(path.vault_relative()).canonicalize() {
            let canonical_root = scope_root
                .canonicalize()
                .map_err(|error| VaultError::Io(error.to_string()))?;
            if !canonical.starts_with(canonical_root) {
                return Err(VaultError::InvalidPath(
                    path.vault_relative().display().to_string(),
                ));
            }
        }
        Ok(())
    }

    pub(super) fn validate_destination_parent(
        &self,
        target: &Path,
        scope: Scope,
    ) -> Result<(), VaultError> {
        let scope_root = self.root.join(match scope {
            Scope::Raw => "sources",
            Scope::Wiki => "wiki",
            Scope::All => unreachable!(),
        });
        let scope_root = scope_root
            .canonicalize()
            .map_err(|error| VaultError::Io(error.to_string()))?;
        if !scope_root.starts_with(&self.root) {
            return Err(VaultError::InvalidPath(scope_root.display().to_string()));
        }
        let parent = target
            .parent()
            .ok_or_else(|| VaultError::InvalidPath(target.display().to_string()))?;
        let mut probe = parent.to_path_buf();
        while !probe.exists() {
            let Some(up) = probe.parent() else { break };
            probe = up.to_path_buf();
        }
        let canonical = probe
            .canonicalize()
            .map_err(|error| VaultError::Io(error.to_string()))?;
        if !canonical.starts_with(scope_root) {
            return Err(VaultError::InvalidPath(target.display().to_string()));
        }
        Ok(())
    }

    pub(super) fn atomic_write(&self, path: &DocPath, content: &[u8]) -> Result<(), VaultError> {
        let target = self.root.join(path.vault_relative());
        self.validate_target(path)?;
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|error| VaultError::Io(error.to_string()))?;
        }
        self.validate_destination_parent(&target, path.scope())?;
        write_atomic(&target, content)
    }

    pub(super) fn snapshot_paths(&self, paths: &[PathBuf]) -> Result<SnapshotSet, VaultError> {
        let files = paths
            .iter()
            .map(|relative| {
                let absolute = self.root.join(relative);
                let content = match std::fs::read(&absolute) {
                    Ok(content) => Some(content),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                    Err(error) => return Err(VaultError::Io(error.to_string())),
                };
                Ok(FileSnapshot {
                    relative: relative.clone(),
                    content,
                })
            })
            .collect::<Result<Vec<_>, VaultError>>()?;
        let mut directories = std::collections::BTreeMap::new();
        for relative in paths {
            let mut parent = relative.parent();
            while let Some(directory) = parent {
                if directory.as_os_str().is_empty() {
                    break;
                }
                let absolute = self.root.join(directory);
                directories
                    .entry(directory.to_path_buf())
                    .or_insert_with(|| DirectorySnapshot {
                        relative: directory.to_path_buf(),
                        existed: absolute.is_dir(),
                    });
                parent = directory.parent();
            }
        }
        Ok(SnapshotSet {
            files,
            directories: directories.into_values().collect(),
        })
    }

    pub(super) fn restore_snapshots(&self, snapshots: SnapshotSet) -> Result<(), VaultError> {
        for snapshot in snapshots.files {
            let absolute = self.root.join(&snapshot.relative);
            if let Some(content) = snapshot.content {
                if let Some(parent) = absolute.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|error| VaultError::Io(error.to_string()))?;
                }
                write_atomic(&absolute, &content)?;
            } else if absolute.exists() {
                std::fs::remove_file(absolute)
                    .map_err(|error| VaultError::Io(error.to_string()))?;
            }
        }
        self.engine.sync_index()?;
        let mut directories = snapshots.directories;
        directories.sort_by_key(|entry| std::cmp::Reverse(entry.relative.components().count()));
        for directory in directories.into_iter().filter(|entry| !entry.existed) {
            let absolute = self.root.join(directory.relative);
            if absolute.is_dir()
                && std::fs::read_dir(&absolute)
                    .map_err(|error| VaultError::Io(error.to_string()))?
                    .next()
                    .is_none()
            {
                std::fs::remove_dir(absolute).map_err(|error| VaultError::Io(error.to_string()))?;
            }
        }
        Ok(())
    }
}
