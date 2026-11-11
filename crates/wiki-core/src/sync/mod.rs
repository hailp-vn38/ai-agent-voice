//! Git-backed synchronization for a writable Wiki vault.

use crate::{DocPath, Scope, Vault, VaultError, VaultEvent};

mod git;
mod merge;
mod types;
use git::hostname;
use git::{GitOutput, find_git, is_unsafe_remote, os_string, run_git, safe_remote};
use merge::collect_conflict_copies;
use std::{
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime},
};
use tokio::sync::broadcast;
pub use types::*;

pub struct VaultSync {
    vault: Arc<Vault>,
    config: SyncConfig,
    git_binary: Option<PathBuf>,
    status: Mutex<SyncStatus>,
    notifications: broadcast::Sender<SyncStatus>,
    vault_events: Mutex<broadcast::Receiver<VaultEvent>>,
    sync_lock: Mutex<()>,
}

impl VaultSync {
    /// Returns the active local branch for a vault, if it already has a Git worktree.
    /// This lets hosts restore their sync configuration without invoking Git themselves.
    pub fn current_branch(vault: &Vault) -> Result<Option<String>, SyncError> {
        let git = find_git(std::env::var_os("PATH").as_deref()).ok_or(SyncError::GitMissing)?;
        match run_git(
            vault.root_path(),
            &["branch", "--show-current"],
            &SyncConfig::default(),
            Some(&git),
            false,
        ) {
            Ok(output) => {
                let branch = output.stdout_string()?.trim().to_string();
                Ok((!branch.is_empty()).then_some(branch))
            }
            Err(SyncError::Git(message)) if message.contains("not a git repository") => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Returns the configured `origin` URL without exposing credentials to host UIs.
    pub fn current_remote(vault: &Vault) -> Result<Option<String>, SyncError> {
        let git = find_git(std::env::var_os("PATH").as_deref()).ok_or(SyncError::GitMissing)?;
        match run_git(
            vault.root_path(),
            &["remote", "get-url", "origin"],
            &SyncConfig::default(),
            Some(&git),
            false,
        ) {
            Ok(output) => {
                let remote = output.stdout_string()?.trim().to_string();
                if is_unsafe_remote(&remote) {
                    return Err(SyncError::UnsafeRemote(safe_remote(&remote)));
                }
                Ok((!remote.is_empty()).then_some(remote))
            }
            Err(SyncError::Git(message))
                if message.contains("not a git repository")
                    || message.contains("No such remote") =>
            {
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }

    pub fn configure(vault: Arc<Vault>, config: SyncConfig) -> Result<Self, SyncError> {
        Self::configure_with_path(vault, config, std::env::var_os("PATH").as_deref())
    }

    fn configure_with_path(
        vault: Arc<Vault>,
        config: SyncConfig,
        search_path: Option<&OsStr>,
    ) -> Result<Self, SyncError> {
        let (notifications, _) = broadcast::channel(32);
        let git_binary = find_git(search_path);
        let mut status = SyncStatus::Idle {
            last_sync: None,
            ahead: 0,
            behind: 0,
        };
        let sync = Self {
            vault_events: Mutex::new(vault.subscribe()),
            vault,
            config,
            git_binary: git_binary.clone(),
            status: Mutex::new(status.clone()),
            notifications,
            sync_lock: Mutex::new(()),
        };

        if !sync.vault.is_writable() {
            status = SyncStatus::Disabled {
                reason: DisabledReason::ReadOnly,
            };
            sync.set_status(status);
            return Ok(sync);
        }
        if git_binary.is_none() {
            sync.set_status(SyncStatus::Disabled {
                reason: DisabledReason::NoGit,
            });
            return Ok(sync);
        }
        sync.git(&["--version"], false)?;
        if let Some(remote) = &sync.config.remote_url {
            if is_unsafe_remote(remote) {
                return Err(SyncError::UnsafeRemote(safe_remote(remote)));
            }
        }
        sync.prepare_repository()?;
        let remote = sync.remote_url()?;
        let Some(remote) = remote else {
            sync.set_status(SyncStatus::Disabled {
                reason: DisabledReason::NotConfigured,
            });
            return Ok(sync);
        };
        if is_unsafe_remote(&remote) {
            return Err(SyncError::UnsafeRemote(safe_remote(&remote)));
        }
        match sync.git(
            &["ls-remote", "--heads", "origin", &sync.config.branch],
            true,
        ) {
            Ok(_) => {}
            Err(error @ SyncError::Auth(_)) | Err(error @ SyncError::Network(_)) => {
                sync.set_status(SyncStatus::Error {
                    kind: error.kind(),
                    message: error.to_string(),
                });
                // Keep the configured synchronizer alive. Hosts need its error status and
                // must be able to retry after credentials/network recover without restarting.
            }
            Err(error) => return Err(error),
        }
        let _ = remote;
        Ok(sync)
    }

    pub fn status(&self) -> SyncStatus {
        self.status
            .lock()
            .map(|status| status.clone())
            .unwrap_or_else(|_| SyncStatus::Error {
                kind: SyncErrorKind::Git,
                message: "sync status lock poisoned".into(),
            })
    }

    /// Re-evaluates lock-dependent status without starting network synchronization.
    pub fn refresh_status(&self) -> Result<SyncStatus, SyncError> {
        let status = if !self.vault.is_writable() {
            SyncStatus::Disabled {
                reason: DisabledReason::ReadOnly,
            }
        } else if self.git_binary.is_none() {
            SyncStatus::Disabled {
                reason: DisabledReason::NoGit,
            }
        } else if self.remote_url()?.is_none() {
            SyncStatus::Disabled {
                reason: DisabledReason::NotConfigured,
            }
        } else {
            SyncStatus::Idle {
                last_sync: None,
                ahead: 0,
                behind: 0,
            }
        };
        self.set_status(status.clone());
        Ok(status)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<SyncStatus> {
        self.notifications.subscribe()
    }

    /// Timeout shared with hosts that need to acquire a whole-vault write gate.
    pub fn command_timeout(&self) -> Duration {
        self.config.command_timeout
    }

    pub fn sync_now(&self) -> Result<SyncReport, SyncError> {
        let _sync_guard = self
            .sync_lock
            .lock()
            .map_err(|_| SyncError::Git("sync lock poisoned".into()))?;
        if !self.vault.is_writable() {
            self.set_status(SyncStatus::Disabled {
                reason: DisabledReason::ReadOnly,
            });
            return Err(SyncError::ReadOnly);
        }
        if self.remote_url()?.is_none() {
            self.set_status(SyncStatus::Disabled {
                reason: DisabledReason::NotConfigured,
            });
            return Err(SyncError::Git("remote origin is not configured".into()));
        }
        self.set_status(SyncStatus::Syncing);
        let result = self.sync_once();
        match result {
            Ok((report, ahead, behind)) => {
                let copies = self.existing_conflict_copies();
                if copies.is_empty() {
                    self.set_status(SyncStatus::Idle {
                        last_sync: Some(SystemTime::now()),
                        ahead,
                        behind,
                    });
                } else {
                    self.set_status(SyncStatus::Conflicted { copies });
                }
                Ok(report)
            }
            Err(error) => {
                self.set_status(SyncStatus::Error {
                    kind: error.kind(),
                    message: error.to_string(),
                });
                Err(error)
            }
        }
    }

    fn sync_once(&self) -> Result<(SyncReport, u32, u32), SyncError> {
        let mut report = SyncReport::default();
        let commit_origins = self.change_origins();
        let mut conflicts = Vec::new();
        let mut inner_error = None;
        let operation = self.vault.exclusive_external(|root| {
            let result = (|| -> Result<Vec<PathBuf>, SyncError> {
                let before = self.git_at(root, &["status", "--porcelain"], false)?;
                if !before.stdout.is_empty() {
                    let candidates = self.git_at(
                        root,
                        &["diff", "--name-only", "--diff-filter=ACMRTUXB", "-z"],
                        false,
                    )?;
                    let unstaged = self.git_at(
                        root,
                        &["ls-files", "--others", "--exclude-standard", "-z"],
                        false,
                    )?;
                    let mut paths: std::collections::BTreeSet<PathBuf> = candidates
                        .stdout
                        .split(|byte| *byte == 0)
                        .chain(unstaged.stdout.split(|byte| *byte == 0))
                        .filter(|entry| !entry.is_empty())
                        .map(|entry| PathBuf::from(os_string(entry)))
                        .collect();
                    let staged =
                        self.git_at(root, &["diff", "--cached", "--name-only", "-z"], false)?;
                    paths.extend(
                        staged
                            .stdout
                            .split(|byte| *byte == 0)
                            .filter(|entry| !entry.is_empty())
                            .map(|entry| PathBuf::from(os_string(entry))),
                    );
                    for relative in paths {
                        let full = root.join(&relative);
                        if let Ok(metadata) = fs::symlink_metadata(&full) {
                            if metadata.file_type().is_file()
                                && metadata.len() > self.config.max_file_bytes
                            {
                                return Err(SyncError::FileTooLarge {
                                    path: relative.to_string_lossy().into_owned(),
                                    bytes: metadata.len(),
                                });
                            }
                        }
                    }
                    self.git_at(root, &["add", "-A"], false)?;
                    let staged =
                        self.git_at(root, &["diff", "--cached", "--name-only", "-z"], false)?;
                    let committed = staged
                        .stdout
                        .split(|byte| *byte == 0)
                        .filter(|item| !item.is_empty())
                        .count();
                    if committed > 0 {
                        report.committed = committed;
                        let message = format!(
                            "vault: {committed} files from {}\n\nOrigin: {}",
                            hostname(),
                            commit_origins
                        );
                        self.git_at(root, &["commit", "-m", &message], false)?;
                    }
                }
                match self.fetch_merge_at(root) {
                    Ok((mut paths, copies)) => {
                        conflicts = copies;
                        self.vault
                            .reindex_exclusive_external()
                            .map_err(|error| SyncError::Vault(error.to_string()))?;
                        if self.commit_derived_index_if_changed(root)? {
                            paths.push(PathBuf::from("wiki/index.md"));
                        }
                        Ok(paths)
                    }
                    Err(error) => Err(error),
                }
            })();
            match result {
                Ok(paths) => Ok(paths),
                Err(error) => {
                    let message = format!("wiki_sync_error:{}", error);
                    inner_error = Some(error);
                    Err(VaultError::Io(message))
                }
            }
        });
        let changed = match operation {
            Ok(paths) => paths,
            Err(_) => {
                return Err(inner_error
                    .unwrap_or_else(|| SyncError::Git("vault sync operation failed".into())));
            }
        };
        report.pulled = changed;
        report.conflicts = conflicts;
        match self.git(&["push", "-u", "origin", &self.config.branch], true) {
            Ok(_) => report.pushed = true,
            Err(SyncError::Git(message)) if message.contains("rejected") => {
                let mut retry_error = None;
                let mut retry_conflicts = Vec::new();
                let retry = self
                    .vault
                    .exclusive_external(|root| match self.fetch_merge_at(root) {
                        Ok((mut paths, copies)) => {
                            retry_conflicts = copies;
                            self.vault
                                .reindex_exclusive_external()
                                .map_err(|error| VaultError::Io(error.to_string()))?;
                            if self
                                .commit_derived_index_if_changed(root)
                                .map_err(|error| VaultError::Io(error.to_string()))?
                            {
                                paths.push(PathBuf::from("wiki/index.md"));
                            }
                            Ok(paths)
                        }
                        Err(error) => {
                            retry_error = Some(error);
                            Err(VaultError::Io("wiki sync retry failed".into()))
                        }
                    });
                match retry {
                    Ok(paths) => {
                        report.pulled.extend(paths);
                        report.conflicts.extend(retry_conflicts);
                    }
                    Err(_) => {
                        return Err(retry_error.unwrap_or_else(|| {
                            SyncError::Git("push retry could not update from origin".into())
                        }));
                    }
                }
                self.git(&["push", "-u", "origin", &self.config.branch], true)?;
                report.pushed = true;
            }
            Err(error) => return Err(error),
        }
        let (ahead, behind) = self.ahead_behind()?;
        Ok((report, ahead, behind))
    }

    fn existing_conflict_copies(&self) -> Vec<DocPath> {
        let mut copies = Vec::new();
        collect_conflict_copies(
            Scope::Raw,
            &self.vault_root().join("sources"),
            &self.vault_root().join("sources"),
            &mut copies,
        );
        collect_conflict_copies(
            Scope::Wiki,
            &self.vault_root().join("wiki"),
            &self.vault_root().join("wiki"),
            &mut copies,
        );
        copies.sort_by(|left, right| left.vault_relative().cmp(&right.vault_relative()));
        copies.dedup();
        copies
    }

    fn prepare_repository(&self) -> Result<(), SyncError> {
        let root = self.vault_root();
        let initialized = self
            .git(&["rev-parse", "--is-inside-work-tree"], false)
            .is_ok();
        if !initialized {
            self.git_at(&root, &["init", "-b", &self.config.branch], false)?;
        }
        self.ensure_file(
            ".gitignore",
            &[
                ".wiki/",
                ".wiki-desktop.lock",
                "*.tmp",
                "*.temp",
                ".obsidian/workspace*.json",
                ".DS_Store",
            ],
        )?;
        self.ensure_file(".gitattributes", &["wiki/log.md merge=union"])?;
        self.git(
            &[
                "rm",
                "--cached",
                "-r",
                "--ignore-unmatch",
                ".wiki",
                ".wiki-desktop.lock",
            ],
            false,
        )?;
        if let Some(remote) = &self.config.remote_url {
            let existing = self.git(&["remote", "get-url", "origin"], false);
            if existing.is_ok() {
                self.git(&["remote", "set-url", "origin", remote], false)?;
            } else {
                self.git(&["remote", "add", "origin", remote], false)?;
            }
        }
        let current = self
            .git(&["branch", "--show-current"], false)?
            .stdout_string()?
            .trim()
            .to_string();
        if current != self.config.branch {
            if self
                .git(
                    &[
                        "show-ref",
                        "--verify",
                        "--quiet",
                        &format!("refs/heads/{}", self.config.branch),
                    ],
                    false,
                )
                .is_ok()
            {
                self.git(&["checkout", &self.config.branch], false)?;
            } else {
                self.git(&["checkout", "-b", &self.config.branch], false)?;
            }
        }
        Ok(())
    }

    fn ensure_file(&self, relative: &str, required: &[&str]) -> Result<(), SyncError> {
        let path = self.vault_root().join(relative);
        let current = match fs::read_to_string(&path) {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => return Err(SyncError::Git(error.to_string())),
        };
        let mut lines: Vec<String> = current.lines().map(ToString::to_string).collect();
        for line in required {
            if !lines.iter().any(|existing| existing.trim() == *line) {
                lines.push((*line).into());
            }
        }
        let next = format!("{}\n", lines.join("\n"));
        if next != current {
            fs::write(path, next).map_err(|error| SyncError::Git(error.to_string()))?;
        }
        Ok(())
    }

    fn remote_url(&self) -> Result<Option<String>, SyncError> {
        if let Some(remote) = &self.config.remote_url {
            return Ok(Some(remote.clone()));
        }
        match self.git(&["remote", "get-url", "origin"], false) {
            Ok(output) => Ok(Some(output.stdout_string()?.trim().to_string())),
            Err(SyncError::Git(_)) => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn ahead_behind(&self) -> Result<(u32, u32), SyncError> {
        let output = self.git(
            &[
                "rev-list",
                "--left-right",
                "--count",
                &format!("HEAD...origin/{}", self.config.branch),
            ],
            false,
        )?;
        let values = output.stdout_string()?;
        let mut parts = values
            .split_whitespace()
            .filter_map(|part| part.parse::<u32>().ok());
        Ok((
            parts.next().unwrap_or_default(),
            parts.next().unwrap_or_default(),
        ))
    }

    fn set_status(&self, status: SyncStatus) {
        if let Ok(mut current) = self.status.lock() {
            *current = status.clone();
            let _ = self.notifications.send(status);
        }
    }

    fn change_origins(&self) -> String {
        let Ok(mut events) = self.vault_events.lock() else {
            return "unknown".into();
        };
        let mut origins = std::collections::BTreeSet::new();
        loop {
            match events.try_recv() {
                Ok(VaultEvent::Changed { origin, .. }) => {
                    origins.insert(match origin {
                        crate::Origin::Ui => "ui",
                        crate::Origin::Mcp => "mcp",
                        crate::Origin::Server => "server",
                    });
                }
                Ok(_) => {}
                Err(broadcast::error::TryRecvError::Lagged(_)) => continue,
                Err(
                    broadcast::error::TryRecvError::Empty | broadcast::error::TryRecvError::Closed,
                ) => break,
            }
        }
        if origins.is_empty() {
            "manual".into()
        } else {
            origins.into_iter().collect::<Vec<_>>().join(", ")
        }
    }

    fn vault_root(&self) -> PathBuf {
        self.vault.root_path().to_path_buf()
    }

    fn git(&self, args: &[&str], classify: bool) -> Result<GitOutput, SyncError> {
        self.git_at(&self.vault_root(), args, classify)
    }

    fn git_at(&self, root: &Path, args: &[&str], classify: bool) -> Result<GitOutput, SyncError> {
        run_git(
            root,
            args,
            &self.config,
            self.git_binary.as_deref(),
            classify,
        )
    }
}

#[cfg(test)]
mod tests;
