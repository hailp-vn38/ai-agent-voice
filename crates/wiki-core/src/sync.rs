//! Git-backed synchronization for a writable Wiki vault.

use crate::{DocPath, Scope, Vault, VaultError, VaultEvent};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant, SystemTime},
};
use thiserror::Error;
use tokio::sync::broadcast;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConfig {
    pub remote_url: Option<String>,
    pub branch: String,
    pub author: Option<GitAuthor>,
    pub auth: GitAuth,
    pub max_file_bytes: u64,
    #[serde(with = "duration_serde")]
    pub command_timeout: Duration,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            remote_url: None,
            branch: "main".into(),
            author: None,
            auth: GitAuth::System,
            max_file_bytes: 50 * 1024 * 1024,
            command_timeout: Duration::from_secs(60),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitAuthor {
    pub name: String,
    pub email: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GitAuth {
    System,
    SshKey { key: PathBuf, known_hosts: PathBuf },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisabledReason {
    NoGit,
    NotConfigured,
    ReadOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeferReason {
    IngestRunning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncErrorKind {
    GitMissing,
    ReadOnly,
    Auth,
    Network,
    Timeout,
    FileTooLarge,
    UnsafeRemote,
    Git,
    Vault,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SyncStatus {
    Disabled {
        reason: DisabledReason,
    },
    Idle {
        #[serde(with = "option_system_time")]
        last_sync: Option<SystemTime>,
        ahead: u32,
        behind: u32,
    },
    Syncing,
    Deferred {
        reason: DeferReason,
        #[serde(with = "system_time")]
        retry_at: SystemTime,
    },
    Conflicted {
        copies: Vec<DocPath>,
    },
    Error {
        kind: SyncErrorKind,
        message: String,
    },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncReport {
    pub committed: usize,
    pub pulled: Vec<DocPath>,
    pub conflicts: Vec<DocPath>,
    pub pushed: bool,
}

#[derive(Debug, Error, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "detail", rename_all = "snake_case")]
pub enum SyncError {
    #[error("git is not installed or not available on PATH")]
    GitMissing,
    #[error("This process does not hold the vault write lock")]
    ReadOnly,
    #[error("Git authentication failed: {0}")]
    Auth(String),
    #[error("Git network operation failed: {0}")]
    Network(String),
    #[error("Git command timed out: {command}")]
    Timeout { command: String },
    #[error("File is larger than the configured limit ({bytes} bytes): {path}")]
    FileTooLarge { path: String, bytes: u64 },
    #[error("Unsafe remote URL: {0}")]
    UnsafeRemote(String),
    #[error("Git operation failed: {0}")]
    Git(String),
    #[error("Vault operation failed: {0}")]
    Vault(String),
}

impl SyncError {
    pub fn kind(&self) -> SyncErrorKind {
        match self {
            Self::GitMissing => SyncErrorKind::GitMissing,
            Self::ReadOnly => SyncErrorKind::ReadOnly,
            Self::Auth(_) => SyncErrorKind::Auth,
            Self::Network(_) => SyncErrorKind::Network,
            Self::Timeout { .. } => SyncErrorKind::Timeout,
            Self::FileTooLarge { .. } => SyncErrorKind::FileTooLarge,
            Self::UnsafeRemote(_) => SyncErrorKind::UnsafeRemote,
            Self::Git(_) => SyncErrorKind::Git,
            Self::Vault(_) => SyncErrorKind::Vault,
        }
    }
}

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
                return Err(error);
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

    pub fn subscribe(&self) -> broadcast::Receiver<SyncStatus> {
        self.notifications.subscribe()
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
                let candidates = self.git_at(
                    root,
                    &[
                        "ls-files",
                        "--cached",
                        "--others",
                        "--exclude-standard",
                        "-z",
                    ],
                    false,
                )?;
                for raw in candidates
                    .stdout
                    .split(|byte| *byte == 0)
                    .filter(|entry| !entry.is_empty())
                {
                    let relative = PathBuf::from(os_string(raw));
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
                let before = self.git_at(root, &["status", "--porcelain"], false)?;
                if !before.stdout.is_empty() {
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

    fn fetch_merge_at(&self, root: &Path) -> Result<(Vec<PathBuf>, Vec<DocPath>), SyncError> {
        let old = self
            .git_at(root, &["rev-parse", "HEAD"], false)?
            .stdout_string()?
            .trim()
            .to_string();
        let branch = self.config.branch.clone();
        match self.git_at(root, &["fetch", "origin", &branch], true) {
            Ok(_) => {}
            Err(SyncError::Git(message)) if message.contains("couldn't find remote ref") => {}
            Err(error) => return Err(error),
        }
        let remote_ref = format!("origin/{branch}");
        let remote_ref_path = format!("refs/remotes/{remote_ref}");
        let mut conflicts = Vec::new();
        if self
            .git_at(
                root,
                &["show-ref", "--verify", "--quiet", &remote_ref_path],
                false,
            )
            .is_ok()
        {
            if self
                .git_at(root, &["merge-base", "HEAD", &remote_ref], false)
                .is_ok()
            {
                if let Err(merge_error) =
                    self.git_at(root, &["merge", "--no-edit", &remote_ref], true)
                {
                    let unmerged = match self.unmerged_paths(root) {
                        Ok(paths) => paths,
                        Err(error) => {
                            let _ = self.git_at(root, &["merge", "--abort"], false);
                            return Err(error);
                        }
                    };
                    if unmerged.is_empty() {
                        let _ = self.git_at(root, &["merge", "--abort"], false);
                        return Err(merge_error);
                    }
                    conflicts = self.resolve_conflicted_merge(root, &unmerged)?;
                }
            } else {
                return Err(SyncError::Git(
                    "local and remote histories are unrelated".into(),
                ));
            }
        }
        let diff = self.git_at(root, &["diff", "--name-only", "-z", &old, "HEAD"], false)?;
        Ok((
            diff.stdout
                .split(|byte| *byte == 0)
                .filter_map(|raw| {
                    if raw.is_empty() {
                        return None;
                    }
                    let relative = PathBuf::from(os_string(raw));
                    doc_path_for(&relative).map(|_| relative)
                })
                .collect(),
            conflicts,
        ))
    }

    fn unmerged_paths(&self, root: &Path) -> Result<Vec<PathBuf>, SyncError> {
        let output = self.git_at(
            root,
            &["diff", "--name-only", "--diff-filter=U", "-z"],
            false,
        )?;
        Ok(output
            .stdout
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
            .map(os_string)
            .map(PathBuf::from)
            .collect())
    }

    fn resolve_conflicted_merge(
        &self,
        root: &Path,
        unmerged: &[PathBuf],
    ) -> Result<Vec<DocPath>, SyncError> {
        let mut created_copies = Vec::new();
        let resolution = (|| {
            let mut copies = Vec::new();
            let mut reindex = false;
            let host = self.remote_host(root);
            let date = utc_date(SystemTime::now());
            for relative in unmerged {
                let relative_text = relative.to_string_lossy().into_owned();
                let ours = self.blob_at_stage(root, 2, &relative_text);
                let theirs = self.blob_at_stage(root, 3, &relative_text);

                if relative == Path::new("wiki/log.md") {
                    let mut merged = ours.clone().unwrap_or_default();
                    if !merged.is_empty() && !merged.ends_with(b"\n") {
                        merged.push(b'\n');
                    }
                    if let Some(remote) = theirs {
                        merged.extend_from_slice(&remote);
                    }
                    fs::write(root.join(relative), merged)
                        .map_err(|error| SyncError::Git(error.to_string()))?;
                    continue;
                }

                if relative == Path::new("wiki/index.md") {
                    if let Some(local) = ours {
                        fs::write(root.join(relative), local)
                            .map_err(|error| SyncError::Git(error.to_string()))?;
                    } else if root.join(relative).exists() {
                        fs::remove_file(root.join(relative))
                            .map_err(|error| SyncError::Git(error.to_string()))?;
                    }
                    reindex = true;
                    continue;
                }

                match (ours, theirs) {
                    (Some(local), Some(remote)) => {
                        let copy = conflict_copy_path(root, relative, &date, &host);
                        fs::write(root.join(&copy), remote)
                            .map_err(|error| SyncError::Git(error.to_string()))?;
                        created_copies.push(copy.clone());
                        if let Some(path) = doc_path_for(&copy) {
                            copies.push(path);
                        }
                        fs::write(root.join(relative), local)
                            .map_err(|error| SyncError::Git(error.to_string()))?;
                    }
                    (Some(local), None) => {
                        fs::write(root.join(relative), local)
                            .map_err(|error| SyncError::Git(error.to_string()))?;
                    }
                    (None, Some(remote)) => {
                        fs::write(root.join(relative), remote)
                            .map_err(|error| SyncError::Git(error.to_string()))?;
                    }
                    (None, None) => {
                        let path = root.join(relative);
                        if path.exists() {
                            fs::remove_file(path)
                                .map_err(|error| SyncError::Git(error.to_string()))?;
                        }
                    }
                }
                let _ = relative_text;
            }

            if reindex {
                self.vault
                    .reindex_exclusive_external()
                    .map_err(|error| SyncError::Vault(error.to_string()))?;
            }
            self.git_at(root, &["add", "-A"], false)?;
            self.git_at(root, &["commit", "--no-edit"], true)?;
            Ok(copies)
        })();

        match resolution {
            Ok(copies) => Ok(copies),
            Err(error) => {
                let _ = self.git_at(root, &["merge", "--abort"], false);
                for path in created_copies {
                    let _ = fs::remove_file(root.join(path));
                }
                Err(error)
            }
        }
    }

    fn blob_at_stage(&self, root: &Path, stage: u8, relative: &str) -> Option<Vec<u8>> {
        self.git_at(root, &["show", &format!(":{stage}:{relative}")], false)
            .ok()
            .map(|output| output.stdout)
    }

    fn commit_derived_index_if_changed(&self, root: &Path) -> Result<bool, SyncError> {
        let status = self.git_at(root, &["status", "--porcelain"], false)?;
        if status.stdout.is_empty() {
            return Ok(false);
        }
        self.git_at(root, &["add", "-A"], false)?;
        let staged = self.git_at(root, &["diff", "--cached", "--name-only", "-z"], false)?;
        if staged.stdout.is_empty() {
            return Ok(false);
        }
        self.git_at(
            root,
            &[
                "commit",
                "-m",
                &format!("vault: rebuild derived index from {}", hostname()),
                "-m",
                "Origin: sync",
            ],
            false,
        )?;
        Ok(true)
    }

    fn remote_host(&self, root: &Path) -> String {
        let Ok(output) = self.git_at(root, &["show", "-s", "--format=%s", "MERGE_HEAD"], false)
        else {
            return "remote".into();
        };
        let subject = String::from_utf8_lossy(&output.stdout);
        let host = subject
            .trim()
            .split_once(" files from ")
            .map(|(_, host)| host)
            .filter(|host| !host.is_empty())
            .unwrap_or("remote");
        safe_filename_part(host)
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
            } else if self.git(&["rev-parse", "--verify", "HEAD"], false).is_ok() {
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

fn run_git(
    root: &Path,
    args: &[&str],
    config: &SyncConfig,
    git_binary: Option<&Path>,
    classify: bool,
) -> Result<GitOutput, SyncError> {
    let Some(git_binary) = git_binary else {
        return Err(SyncError::GitMissing);
    };
    let mut command = Command::new(git_binary);
    command
        .current_dir(root)
        .arg("-c")
        .arg(format!("safe.directory={}", root.display()))
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "")
        .env("LC_ALL", "C")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Git global configuration is disabled for a deploy key so no ambient credential helper is consulted.
    if let Some((ssh, global_config, no_system_config)) = ssh_environment(&config.auth) {
        command
            .env("GIT_SSH_COMMAND", ssh)
            .env("GIT_CONFIG_GLOBAL", global_config)
            .env("GIT_CONFIG_NOSYSTEM", no_system_config)
            .env_remove("HOME");
    }
    if let Some(author) = &config.author {
        command
            .env("GIT_AUTHOR_NAME", &author.name)
            .env("GIT_AUTHOR_EMAIL", &author.email)
            .env("GIT_COMMITTER_NAME", &author.name)
            .env("GIT_COMMITTER_EMAIL", &author.email);
    }
    let mut child = command.spawn().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            SyncError::GitMissing
        } else {
            SyncError::Git(error.to_string())
        }
    })?;
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let stdout_reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = std::io::Read::read_to_end(&mut std::io::BufReader::new(stdout), &mut bytes);
        bytes
    });
    let stderr_reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = std::io::Read::read_to_end(&mut std::io::BufReader::new(stderr), &mut bytes);
        bytes
    });
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| SyncError::Git(error.to_string()))?
        {
            break status;
        }
        if started.elapsed() >= config.command_timeout {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdout_reader.join();
            let _ = stderr_reader.join();
            return Err(SyncError::Timeout {
                command: args.first().copied().unwrap_or("git").into(),
            });
        }
        thread::sleep(Duration::from_millis(10));
    };
    let stdout = stdout_reader.join().unwrap_or_default();
    let stderr = sanitize(&String::from_utf8_lossy(
        &stderr_reader.join().unwrap_or_default(),
    ));
    if !status.success() {
        let message = if stderr.is_empty() {
            "git exited unsuccessfully".into()
        } else {
            stderr
        };
        let lowered = message.to_ascii_lowercase();
        let error = if lowered.contains("authentication failed")
            || lowered.contains("permission denied (publickey)")
            || lowered.contains("could not read username")
            || lowered.contains("access denied")
        {
            SyncError::Auth(message)
        } else if lowered.contains("could not resolve host")
            || lowered.contains("network is unreachable")
            || lowered.contains("connection timed out")
            || lowered.contains("failed to connect")
        {
            SyncError::Network(message)
        } else {
            SyncError::Git(message)
        };
        let _ = classify;
        return Err(error);
    }
    Ok(GitOutput { stdout })
}

struct GitOutput {
    stdout: Vec<u8>,
}
impl GitOutput {
    fn stdout_string(&self) -> Result<String, SyncError> {
        String::from_utf8(self.stdout.clone()).map_err(|error| SyncError::Git(error.to_string()))
    }
}

fn doc_path_for(path: &Path) -> Option<DocPath> {
    let (scope, prefix) = if path.starts_with("sources") {
        (Scope::Raw, "sources")
    } else if path.starts_with("wiki") {
        (Scope::Wiki, "wiki")
    } else {
        return None;
    };
    DocPath::parse(scope, path.strip_prefix(prefix).ok()?).ok()
}

fn conflict_copy_path(root: &Path, relative: &Path, date: &str, host: &str) -> PathBuf {
    let parent = relative.parent().unwrap_or_else(|| Path::new("."));
    let file_stem = relative
        .file_stem()
        .and_then(OsStr::to_str)
        .unwrap_or("page");
    let extension = relative.extension().and_then(OsStr::to_str).unwrap_or("md");
    let base = format!("{file_stem} (conflict {date} {host})");
    let mut candidate = parent.join(format!("{base}.{extension}"));
    let mut suffix = 2;
    while root.join(&candidate).exists() {
        candidate = parent.join(format!("{base} {suffix}.{extension}"));
        suffix += 1;
    }
    candidate
}

fn safe_filename_part(value: &str) -> String {
    let sanitized: String = value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '-'
            }
        })
        .collect();
    let sanitized = sanitized.trim_matches(['-', '.']);
    if sanitized.is_empty() {
        "remote".into()
    } else {
        sanitized.to_string()
    }
}

fn utc_date(time: SystemTime) -> String {
    let seconds = time
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days = (seconds / 86_400) as i64;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

fn collect_conflict_copies(scope: Scope, root: &Path, directory: &Path, copies: &mut Vec<DocPath>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            collect_conflict_copies(scope, root, &path, copies);
            continue;
        }
        if !path
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(|name| name.contains(" (conflict "))
        {
            continue;
        }
        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        if let Ok(doc_path) = DocPath::parse(scope, relative) {
            copies.push(doc_path);
        }
    }
}
#[cfg(unix)]
fn os_string(bytes: &[u8]) -> std::ffi::OsString {
    use std::os::unix::ffi::OsStringExt;
    std::ffi::OsString::from_vec(bytes.to_vec())
}
#[cfg(not(unix))]
fn os_string(bytes: &[u8]) -> std::ffi::OsString {
    std::ffi::OsString::from(String::from_utf8_lossy(bytes).into_owned())
}
fn hostname() -> String {
    #[cfg(unix)]
    {
        let mut bytes = [0u8; 256];
        let result = unsafe { libc::gethostname(bytes.as_mut_ptr().cast(), bytes.len()) };
        if result == 0 {
            let end = bytes
                .iter()
                .position(|byte| *byte == 0)
                .unwrap_or(bytes.len());
            let value = String::from_utf8_lossy(&bytes[..end]).trim().to_string();
            if !value.is_empty() {
                return value;
            }
        }
    }
    std::env::var("HOSTNAME")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unknown-host".into())
}
fn is_unsafe_remote(remote: &str) -> bool {
    let has_userinfo = remote.split_once("://").is_some_and(|(_, rest)| {
        rest.split('/')
            .next()
            .is_some_and(|authority| authority.contains('@'))
    });
    let lower = remote.to_ascii_lowercase();
    has_userinfo
        || ["token=", "password=", "passwd=", "access_token=", "auth="]
            .iter()
            .any(|marker| lower.contains(marker))
}
fn safe_remote(remote: &str) -> String {
    let redacted = if let Some((base, query)) = remote.split_once('?') {
        let lower = query.to_ascii_lowercase();
        if ["token=", "password=", "passwd=", "access_token=", "auth="]
            .iter()
            .any(|marker| lower.contains(marker))
        {
            format!("{base}?[redacted]")
        } else {
            remote.to_string()
        }
    } else {
        remote.to_string()
    };
    redacted
        .split_once("://")
        .map(|(scheme, rest)| {
            format!(
                "{scheme}://{}",
                rest.rsplit_once('@').map(|(_, host)| host).unwrap_or(rest)
            )
        })
        .unwrap_or_else(|| "remote URL contains credentials".into())
}
fn sanitize(message: &str) -> String {
    let mut output = String::new();
    for token in message.split_whitespace() {
        if !output.is_empty() {
            output.push(' ');
        }
        let token = token.trim_matches(|ch: char| matches!(ch, ',' | ';' | ')' | '(' | '"' | '\''));
        if token.contains("://") && token.contains('@') {
            output.push_str(&safe_remote(token));
        } else if token.to_ascii_lowercase().contains("token=")
            || token.to_ascii_lowercase().contains("password=")
        {
            output.push_str("[redacted]");
        } else {
            output.push_str(token);
        }
    }
    output
}
fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "'\\''"))
}

fn ssh_command(key: &Path, known_hosts: &Path) -> String {
    format!(
        "ssh -i {} -o IdentitiesOnly=yes -o UserKnownHostsFile={} -o StrictHostKeyChecking=yes",
        shell_quote(key),
        shell_quote(known_hosts)
    )
}

fn ssh_environment(auth: &GitAuth) -> Option<(String, &'static str, &'static str)> {
    match auth {
        GitAuth::System => None,
        GitAuth::SshKey { key, known_hosts } => {
            Some((ssh_command(key, known_hosts), "/dev/null", "1"))
        }
    }
}

fn find_git(search_path: Option<&OsStr>) -> Option<PathBuf> {
    let path = search_path?;
    std::env::split_paths(path)
        .map(|directory| directory.join("git"))
        .find(|candidate| is_executable(candidate))
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path)
        .map(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

mod duration_serde {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::Duration;
    pub fn serialize<S: Serializer>(value: &Duration, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u64(value.as_secs())
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Duration, D::Error> {
        Ok(Duration::from_secs(u64::deserialize(deserializer)?))
    }
}
mod system_time {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};
    pub fn serialize<S: Serializer>(value: &SystemTime, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u64(
            value
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        )
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<SystemTime, D::Error> {
        Ok(UNIX_EPOCH + Duration::from_secs(u64::deserialize(deserializer)?))
    }
}
mod option_system_time {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};
    pub fn serialize<S: Serializer>(
        value: &Option<SystemTime>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        value
            .map(|time| {
                time.duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
            })
            .serialize(serializer)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<SystemTime>, D::Error> {
        Ok(Option::<u64>::deserialize(deserializer)?
            .map(|secs| UNIX_EPOCH + Duration::from_secs(secs)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Change, Origin, ReadWindow};
    use std::io::Write;
    use std::process::Output;

    fn git(root: &Path, args: &[&str]) -> Output {
        let output = Command::new("git")
            .current_dir(root)
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .expect("git is installed for this test");
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }

    fn config(remote: String) -> SyncConfig {
        SyncConfig {
            remote_url: Some(remote),
            author: Some(GitAuthor {
                name: "Wiki Test".into(),
                email: "wiki-test@example.invalid".into(),
            }),
            ..SyncConfig::default()
        }
    }

    fn write_page(vault: &Vault, path: &str, content: &str) {
        vault
            .apply_from(
                Origin::Ui,
                Change::Write {
                    path: DocPath::parse(Scope::Wiki, path).unwrap(),
                    content: content.into(),
                    expect: None,
                },
            )
            .unwrap();
    }

    fn new_bare_remote(root: &Path) -> PathBuf {
        let remote = root.join("remote.git");
        fs::create_dir_all(&remote).unwrap();
        git(&remote, &["init", "--bare", "-b", "main"]);
        remote
    }

    fn skip_without_git() -> bool {
        if Command::new("git").arg("--version").output().is_ok() {
            return false;
        }
        println!("SKIPPED: git is not installed");
        true
    }

    fn assert_clean(root: &Path) {
        let status = git(root, &["status", "--porcelain"]);
        assert!(
            status.stdout.is_empty(),
            "repository should be clean after sync: {}",
            String::from_utf8_lossy(&status.stdout)
        );
    }

    #[test]
    fn two_vaults_exchange_markdown_and_publish_external_event() {
        if skip_without_git() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let remote = new_bare_remote(temp.path());
        let remote_url = remote.to_string_lossy().into_owned();
        let vault_a =
            Arc::new(Vault::open(crate::VaultConfig::new(temp.path().join("vault-a"))).unwrap());
        let sync_a =
            VaultSync::configure(Arc::clone(&vault_a), config(remote_url.clone())).unwrap();
        sync_a.sync_now().unwrap();

        let b_root = temp.path().join("vault-b");
        fs::create_dir_all(&b_root).unwrap();
        git(&b_root, &["clone", &remote_url, "."]);
        let vault_b = Arc::new(Vault::open(crate::VaultConfig::new(&b_root)).unwrap());
        let sync_b = VaultSync::configure(Arc::clone(&vault_b), config(remote_url)).unwrap();
        let mut statuses = sync_b.subscribe();
        let mut events = vault_b.subscribe();
        write_page(
            &vault_a,
            "remote-page.md",
            "# Remote Page\nneedle from another computer\n",
        );
        sync_a.sync_now().unwrap();
        let message =
            String::from_utf8(git(vault_a.root_path(), &["log", "-1", "--format=%B"]).stdout)
                .unwrap();
        assert!(message.contains("Origin: ui"));

        let report = sync_b.sync_now().unwrap();
        assert!(matches!(statuses.try_recv(), Ok(SyncStatus::Syncing)));
        assert!(matches!(statuses.try_recv(), Ok(SyncStatus::Idle { .. })));
        let expected = DocPath::parse(Scope::Wiki, "remote-page.md").unwrap();
        assert!(report.pulled.contains(&expected));
        assert_eq!(
            vault_b
                .read(&expected, ReadWindow::default())
                .unwrap()
                .content,
            "# Remote Page\nneedle from another computer"
        );
        assert!(
            vault_b
                .search("needle")
                .unwrap()
                .iter()
                .any(|hit| hit.path == "wiki/remote-page.md")
        );
        let external = loop {
            match events.try_recv() {
                Ok(VaultEvent::External { paths }) => break paths,
                Ok(_) => continue,
                Err(error) => panic!("expected External event, got {error:?}"),
            }
        };
        assert!(external.iter().any(|path| path == "wiki/remote-page.md"));
    }

    #[test]
    fn concurrent_page_edits_preserve_local_and_create_remote_conflict_copy() {
        if skip_without_git() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let remote = new_bare_remote(temp.path());
        let remote_url = remote.to_string_lossy().into_owned();
        let vault_a =
            Arc::new(Vault::open(crate::VaultConfig::new(temp.path().join("vault-a"))).unwrap());
        let sync_a =
            VaultSync::configure(Arc::clone(&vault_a), config(remote_url.clone())).unwrap();
        write_page(&vault_a, "shared.md", "# Shared\nbase\n");
        sync_a.sync_now().unwrap();

        let b_root = temp.path().join("vault-b");
        fs::create_dir_all(&b_root).unwrap();
        git(&b_root, &["clone", &remote_url, "."]);
        let vault_b = Arc::new(Vault::open(crate::VaultConfig::new(&b_root)).unwrap());
        let sync_b = VaultSync::configure(Arc::clone(&vault_b), config(remote_url)).unwrap();

        write_page(&vault_a, "shared.md", "# Shared\nchange from A\n");
        write_page(&vault_b, "shared.md", "# Shared\nchange from B\n");
        sync_a.sync_now().unwrap();

        let report = sync_b.sync_now().unwrap();
        let shared = DocPath::parse(Scope::Wiki, "shared.md").unwrap();
        assert_eq!(
            vault_b
                .read(&shared, ReadWindow::default())
                .unwrap()
                .content,
            "# Shared\nchange from B"
        );
        assert!(!report.conflicts.is_empty());
        let status = sync_b.status();
        assert!(matches!(status, SyncStatus::Conflicted { .. }));

        let remote_copy = fs::read_dir(vault_b.root_path().join("wiki"))
            .unwrap()
            .filter_map(Result::ok)
            .find(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .contains("shared (conflict ")
            })
            .expect("remote version should be saved as a conflict copy");
        assert_eq!(
            fs::read_to_string(remote_copy.path()).unwrap(),
            "# Shared\nchange from A\n"
        );

        fs::remove_file(remote_copy.path()).unwrap();
        sync_b.sync_now().unwrap();
        assert!(matches!(sync_b.status(), SyncStatus::Idle { .. }));
        assert_clean(vault_a.root_path());
        assert_clean(vault_b.root_path());
    }

    #[test]
    fn concurrent_log_appends_are_unioned_without_conflict_markers() {
        if skip_without_git() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let remote = new_bare_remote(temp.path());
        let remote_url = remote.to_string_lossy().into_owned();
        let vault_a =
            Arc::new(Vault::open(crate::VaultConfig::new(temp.path().join("vault-a"))).unwrap());
        let sync_a =
            VaultSync::configure(Arc::clone(&vault_a), config(remote_url.clone())).unwrap();
        fs::write(vault_a.root_path().join("wiki/log.md"), "# Log\nbase\n").unwrap();
        sync_a.sync_now().unwrap();

        let b_root = temp.path().join("vault-b");
        fs::create_dir_all(&b_root).unwrap();
        git(&b_root, &["clone", &remote_url, "."]);
        let vault_b = Arc::new(Vault::open(crate::VaultConfig::new(&b_root)).unwrap());
        let sync_b = VaultSync::configure(Arc::clone(&vault_b), config(remote_url)).unwrap();

        fs::OpenOptions::new()
            .append(true)
            .open(vault_a.root_path().join("wiki/log.md"))
            .unwrap()
            .write_all(b"from A\n")
            .unwrap();
        fs::OpenOptions::new()
            .append(true)
            .open(vault_b.root_path().join("wiki/log.md"))
            .unwrap()
            .write_all(b"from B\n")
            .unwrap();
        sync_a.sync_now().unwrap();
        sync_b.sync_now().unwrap();

        let log = fs::read_to_string(vault_b.root_path().join("wiki/log.md")).unwrap();
        assert!(log.contains("from A"));
        assert!(log.contains("from B"));
        assert!(!log.contains("<<<<<<<"));
        assert!(!log.contains(">>>>>>>"));
        assert!(matches!(sync_b.status(), SyncStatus::Idle { .. }));
        assert_clean(vault_a.root_path());
        assert_clean(vault_b.root_path());
    }

    #[test]
    fn index_conflict_is_rebuilt_after_remote_pages_merge() {
        if skip_without_git() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let remote = new_bare_remote(temp.path());
        let remote_url = remote.to_string_lossy().into_owned();
        let vault_a =
            Arc::new(Vault::open(crate::VaultConfig::new(temp.path().join("vault-a"))).unwrap());
        let sync_a =
            VaultSync::configure(Arc::clone(&vault_a), config(remote_url.clone())).unwrap();
        sync_a.sync_now().unwrap();

        let b_root = temp.path().join("vault-b");
        fs::create_dir_all(&b_root).unwrap();
        git(&b_root, &["clone", &remote_url, "."]);
        let vault_b = Arc::new(Vault::open(crate::VaultConfig::new(&b_root)).unwrap());
        let sync_b = VaultSync::configure(Arc::clone(&vault_b), config(remote_url)).unwrap();

        write_page(&vault_a, "from-a.md", "# From A\nA content\n");
        write_page(&vault_b, "from-b.md", "# From B\nB content\n");
        sync_a.sync_now().unwrap();
        sync_b.sync_now().unwrap();

        let index = fs::read_to_string(vault_b.root_path().join("wiki/index.md")).unwrap();
        assert!(!index.contains("<<<<<<<"));
        assert!(!index.contains("======="));
        assert!(!index.contains(">>>>>>>"));
        assert!(index.contains("From A"));
        assert!(index.contains("From B"));
        assert_clean(vault_a.root_path());
        assert_clean(vault_b.root_path());
    }

    #[test]
    fn delete_modify_conflict_keeps_the_modified_page() {
        if skip_without_git() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let remote = new_bare_remote(temp.path());
        let remote_url = remote.to_string_lossy().into_owned();
        let vault_a =
            Arc::new(Vault::open(crate::VaultConfig::new(temp.path().join("vault-a"))).unwrap());
        let sync_a =
            VaultSync::configure(Arc::clone(&vault_a), config(remote_url.clone())).unwrap();
        write_page(&vault_a, "delete-modify.md", "# Keep Edited\nbase\n");
        sync_a.sync_now().unwrap();

        let b_root = temp.path().join("vault-b");
        fs::create_dir_all(&b_root).unwrap();
        git(&b_root, &["clone", &remote_url, "."]);
        let vault_b = Arc::new(Vault::open(crate::VaultConfig::new(&b_root)).unwrap());
        let sync_b = VaultSync::configure(Arc::clone(&vault_b), config(remote_url)).unwrap();

        vault_a
            .apply(Change::Delete {
                path: DocPath::parse(Scope::Wiki, "delete-modify.md").unwrap(),
            })
            .unwrap();
        write_page(&vault_b, "delete-modify.md", "# Keep Edited\nB edit\n");
        sync_a.sync_now().unwrap();
        sync_b.sync_now().unwrap();

        let page = DocPath::parse(Scope::Wiki, "delete-modify.md").unwrap();
        assert_eq!(
            vault_b.read(&page, ReadWindow::default()).unwrap().content,
            "# Keep Edited\nB edit"
        );
        assert!(matches!(sync_b.status(), SyncStatus::Idle { .. }));
        assert_clean(vault_a.root_path());
        assert_clean(vault_b.root_path());
    }

    #[test]
    fn sync_ignores_search_projection_and_lock_file() {
        if skip_without_git() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let remote = new_bare_remote(temp.path());
        let vault =
            Arc::new(Vault::open(crate::VaultConfig::new(temp.path().join("vault"))).unwrap());
        let sync = VaultSync::configure(
            Arc::clone(&vault),
            config(remote.to_string_lossy().into_owned()),
        )
        .unwrap();
        sync.sync_now().unwrap();
        let files = String::from_utf8(git(vault.root_path(), &["ls-files"]).stdout).unwrap();
        assert!(
            !files
                .lines()
                .any(|path| path == ".wiki/index.sqlite" || path == ".wiki-desktop.lock")
        );
    }

    #[test]
    fn oversized_markdown_is_rejected_before_creating_a_commit() {
        if skip_without_git() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let remote = new_bare_remote(temp.path());
        let vault =
            Arc::new(Vault::open(crate::VaultConfig::new(temp.path().join("vault"))).unwrap());
        let mut cfg = config(remote.to_string_lossy().into_owned());
        cfg.max_file_bytes = 64 * 1024;
        let sync = VaultSync::configure(Arc::clone(&vault), cfg).unwrap();
        sync.sync_now().unwrap();
        let before =
            String::from_utf8(git(vault.root_path(), &["rev-parse", "HEAD"]).stdout).unwrap();
        write_page(
            &vault,
            "too-large.md",
            &format!("# Big\n{}", "x".repeat(70 * 1024)),
        );
        let error = sync.sync_now().unwrap_err();
        assert!(
            matches!(error, SyncError::FileTooLarge { .. }),
            "unexpected error: {error:?}"
        );
        let after =
            String::from_utf8(git(vault.root_path(), &["rev-parse", "HEAD"]).stdout).unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn unsafe_remote_is_rejected_and_reader_cannot_sync() {
        if skip_without_git() {
            return;
        }
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("vault");
        let writer = Arc::new(Vault::open(crate::VaultConfig::new(&root)).unwrap());
        assert!(matches!(
            VaultSync::configure(Arc::clone(&writer), config("https://u:t@host/r.git".into())),
            Err(SyncError::UnsafeRemote(_))
        ));
        let reader = Arc::new(Vault::open(crate::VaultConfig::new(&root)).unwrap());
        let sync =
            VaultSync::configure(reader, config("https://example.invalid/r.git".into())).unwrap();
        assert!(matches!(
            sync.status(),
            SyncStatus::Disabled {
                reason: DisabledReason::ReadOnly
            }
        ));
        assert!(matches!(sync.sync_now(), Err(SyncError::ReadOnly)));
    }

    #[test]
    fn no_git_disables_sync_and_ssh_arguments_are_headless() {
        let temp = tempfile::tempdir().unwrap();
        let empty_path = temp.path().join("empty-bin");
        fs::create_dir(&empty_path).unwrap();
        let vault = Arc::new(
            Vault::open(crate::VaultConfig::new(temp.path().join("no-git-vault"))).unwrap(),
        );
        let sync = VaultSync::configure_with_path(
            Arc::clone(&vault),
            config("file:///tmp/unused.git".into()),
            Some(empty_path.as_os_str()),
        )
        .unwrap();
        assert!(matches!(
            sync.status(),
            SyncStatus::Disabled {
                reason: DisabledReason::NoGit
            }
        ));
        assert!(matches!(sync.sync_now(), Err(SyncError::GitMissing)));

        let ssh = ssh_command(
            Path::new("/keys/deploy key"),
            Path::new("/keys/known hosts"),
        );
        assert!(ssh.contains("-i '/keys/deploy key'"));
        assert!(ssh.contains("-o IdentitiesOnly=yes"));
        assert!(ssh.contains("-o UserKnownHostsFile='/keys/known hosts'"));
        assert!(ssh.contains("-o StrictHostKeyChecking=yes"));
        let ssh_config = SyncConfig {
            auth: GitAuth::SshKey {
                key: PathBuf::from("/keys/deploy"),
                known_hosts: PathBuf::from("/keys/known_hosts"),
            },
            ..SyncConfig::default()
        };
        let (ssh_command, global_config, no_system_config) =
            ssh_environment(&ssh_config.auth).unwrap();
        assert!(ssh_command.contains("-i '/keys/deploy'"));
        assert_eq!(global_config, "/dev/null");
        assert_eq!(no_system_config, "1");
        let git_binary = find_git(std::env::var_os("PATH").as_deref());
        let version = run_git(
            temp.path(),
            &["--version"],
            &ssh_config,
            git_binary.as_deref(),
            false,
        )
        .unwrap();
        assert!(version.stdout_string().unwrap().starts_with("git version "));
    }
}
