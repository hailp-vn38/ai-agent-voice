//! Merge result paths and conflict-copy handling helpers.

use super::git::{hostname, os_string};
use crate::sync::SyncError;
use crate::{DocPath, Scope};
use std::{
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

pub(super) fn doc_path_for(path: &Path) -> Option<DocPath> {
    let (scope, prefix) = if path.starts_with("sources") {
        (Scope::Raw, "sources")
    } else if path.starts_with("wiki") {
        (Scope::Wiki, "wiki")
    } else {
        return None;
    };
    DocPath::parse(scope, path.strip_prefix(prefix).ok()?).ok()
}

pub(super) fn conflict_copy_path(root: &Path, relative: &Path, date: &str, host: &str) -> PathBuf {
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

pub(super) fn safe_filename_part(value: &str) -> String {
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

pub(super) fn utc_date(time: SystemTime) -> String {
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

pub(super) fn collect_conflict_copies(
    scope: Scope,
    root: &Path,
    directory: &Path,
    copies: &mut Vec<DocPath>,
) {
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

impl super::VaultSync {
    pub(super) fn fetch_merge_at(
        &self,
        root: &Path,
    ) -> Result<(Vec<PathBuf>, Vec<DocPath>), SyncError> {
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

    pub(super) fn unmerged_paths(&self, root: &Path) -> Result<Vec<PathBuf>, SyncError> {
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

    pub(super) fn resolve_conflicted_merge(
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

    pub(super) fn blob_at_stage(&self, root: &Path, stage: u8, relative: &str) -> Option<Vec<u8>> {
        self.git_at(root, &["show", &format!(":{stage}:{relative}")], false)
            .ok()
            .map(|output| output.stdout)
    }

    pub(super) fn commit_derived_index_if_changed(&self, root: &Path) -> Result<bool, SyncError> {
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

    pub(super) fn remote_host(&self, root: &Path) -> String {
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
}
