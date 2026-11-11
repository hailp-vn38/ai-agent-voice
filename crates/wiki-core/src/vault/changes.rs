//! Change validation, execution, and transactional orchestration.

use super::Vault;
use crate::{Applied, Change, DocPath, Origin, Revision, Scope, TextEdit, VaultError, VaultEvent};
use llm_wiki::{CollectionScope, IngestSourceRequest, LogAction};
use std::path::PathBuf;

#[derive(Default)]
struct ChangePlan {
    paths: Vec<PathBuf>,
}

fn apply_text_edits(current: &str, mut edits: Vec<TextEdit>) -> Result<String, VaultError> {
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.start_line));
    let mut lines: Vec<String> = current.lines().map(ToString::to_string).collect();
    for edit in edits {
        if edit.start_line > lines.len()
            || edit.start_line.saturating_add(edit.delete_lines) > lines.len()
        {
            return Err(VaultError::InvalidPath(
                "patch line range is outside the document".into(),
            ));
        }
        lines.splice(
            edit.start_line..edit.start_line + edit.delete_lines,
            edit.insert.lines().map(ToString::to_string),
        );
    }
    let mut result = lines.join("\n");
    if current.ends_with('\n') || !result.is_empty() {
        result.push('\n');
    }
    Ok(result)
}

fn source_title_summary(content: &str, path: &DocPath) -> (String, String) {
    let mut in_frontmatter = false;
    let mut body = Vec::new();
    for line in content.lines() {
        if line.trim() == "---" {
            in_frontmatter = !in_frontmatter;
            continue;
        }
        if !in_frontmatter {
            body.push(line.trim());
        }
    }
    let title = body
        .iter()
        .find_map(|line| line.strip_prefix("# ").map(str::trim))
        .filter(|title| !title.is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            path.relative()
                .file_stem()
                .and_then(|stem| stem.to_str())
                .map(ToString::to_string)
        })
        .unwrap_or_else(|| "Imported Source".into());
    let summary = body
        .iter()
        .map(|line| line.trim_start_matches('#').trim())
        .find(|line| !line.is_empty())
        .unwrap_or(&title)
        .chars()
        .take(180)
        .collect();
    (title, summary)
}

impl Vault {
    pub fn apply(&self, change: Change) -> Result<Applied, VaultError> {
        self.apply_from(Origin::Server, change)
    }

    pub fn apply_from(&self, origin: Origin, change: Change) -> Result<Applied, VaultError> {
        if !self.is_writable() {
            return Err(VaultError::ReadOnly);
        }
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| VaultError::Io("vault write lock poisoned".into()))?;
        let reindex = matches!(change, Change::Reindex { .. });
        let plan = self.prepare_change(&change)?;
        let snapshots = self.snapshot_paths(&plan.paths)?;
        let result = self.execute_change(change, plan);
        let (paths, revision) = match result {
            Ok(result) => result,
            Err(error) => {
                self.restore_snapshots(snapshots)?;
                return Err(error);
            }
        };
        if reindex {
            return Ok(Applied { paths, revision });
        }
        let mut event_paths = paths;
        event_paths.push("wiki/index.md".into());
        event_paths.push("wiki/log.md".into());
        event_paths.sort();
        event_paths.dedup();
        let _ = self.record_self_writes(&event_paths);
        let _ = self.events.send(VaultEvent::Changed {
            paths: event_paths.clone(),
            origin,
        });
        Ok(Applied {
            paths: event_paths,
            revision,
        })
    }

    fn prepare_change(&self, change: &Change) -> Result<ChangePlan, VaultError> {
        let mut plan = ChangePlan::default();
        match change {
            Change::Write { path, expect, .. } | Change::Patch { path, expect, .. } => {
                self.validate_target(path)?;
                let current = match std::fs::read(self.root.join(path.vault_relative())) {
                    Ok(content) => Some(content),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                    Err(error) => return Err(VaultError::Io(error.to_string())),
                };
                if let Some(expected) = expect {
                    let Some(current_content) = current.as_deref() else {
                        return Err(VaultError::NotFound(
                            path.vault_relative().display().to_string(),
                        ));
                    };
                    let current_revision = Revision::of(current_content);
                    if &current_revision != expected {
                        return Err(VaultError::Conflict {
                            current: current_revision,
                        });
                    }
                }
                plan.paths.push(path.vault_relative());
            }
            Change::Move { from, to } => {
                self.validate_target(from)?;
                self.validate_target(to)?;
                if from.scope() != to.scope() {
                    return Err(VaultError::InvalidPath(
                        "move must stay within one space".into(),
                    ));
                }
                let from_disk = self.root.join(from.vault_relative());
                if !from_disk.is_file() {
                    return Err(VaultError::NotFound(
                        from.vault_relative().display().to_string(),
                    ));
                }
                if from != to && self.root.join(to.vault_relative()).exists() {
                    return Err(VaultError::InvalidPath(format!(
                        "move target already exists: {}",
                        to.vault_relative().display()
                    )));
                }
                if to.scope() == Scope::Wiki
                    && self
                        .engine()?
                        .get_backlinks(&from.vault_relative().to_string_lossy())?
                        .iter()
                        .any(|edge| {
                            edge.source_rel_path != "wiki/index.md"
                                && edge.source_rel_path != "wiki/log.md"
                        })
                {
                    return Err(VaultError::InvalidPath(
                        "move would break incoming Wiki Links".into(),
                    ));
                }
                plan.paths
                    .extend([from.vault_relative(), to.vault_relative()]);
            }
            Change::Delete { path } => {
                self.validate_target(path)?;
                plan.paths.push(path.vault_relative());
            }
            Change::ImportRaw { files } => {
                for file in files {
                    if file.path.scope() != Scope::Raw {
                        return Err(VaultError::InvalidPath(
                            "ImportRaw accepts only raw paths".into(),
                        ));
                    }
                    self.validate_target(&file.path)?;
                    plan.paths.push(file.path.vault_relative());
                }
            }
            Change::IngestSource { source, details } => {
                if source.scope() != Scope::Raw {
                    return Err(VaultError::InvalidPath(
                        "IngestSource requires a raw path".into(),
                    ));
                }
                self.validate_target(source)?;
                let source_path = self.root.join(source.vault_relative());
                if !source_path.is_file()
                    && !details
                        .as_ref()
                        .is_some_and(|details| details.raw_source_content.is_some())
                {
                    return Err(VaultError::NotFound(
                        source.vault_relative().display().to_string(),
                    ));
                }
                plan.paths.push(source.vault_relative());
                if let Some(target) = details
                    .as_ref()
                    .and_then(|details| details.target_wiki_path.as_deref())
                {
                    let target = target.trim_start_matches("wiki/");
                    let target = DocPath::parse(Scope::Wiki, target)?;
                    self.validate_target(&target)?;
                    plan.paths.push(target.vault_relative());
                } else {
                    let source_rel = source.vault_relative().to_string_lossy().into_owned();
                    let wiki_rel = self.engine()?.resolve_wiki_path_for_source(&source_rel)?.0;
                    plan.paths.push(PathBuf::from(wiki_rel));
                }
            }
            Change::FileSynthesis { path, .. } => {
                self.validate_target(path)?;
                plan.paths.push(path.vault_relative());
            }
            Change::Reindex { .. } => {}
        }
        if !matches!(change, Change::Reindex { .. }) {
            plan.paths.push(PathBuf::from("wiki/index.md"));
            plan.paths.push(PathBuf::from("wiki/log.md"));
        }
        Ok(plan)
    }

    fn execute_change(
        &self,
        change: Change,
        plan: ChangePlan,
    ) -> Result<(Vec<String>, Option<Revision>), VaultError> {
        let mut paths = Vec::new();
        let mut revision = None;
        let specialized_indexing = matches!(
            &change,
            Change::IngestSource { .. } | Change::FileSynthesis { .. }
        );
        match change {
            Change::Write { path, content, .. } => {
                self.atomic_write(&path, content.as_bytes())?;
                revision = Some(Revision::of(content.as_bytes()));
                paths.push(path.vault_relative().display().to_string());
            }
            Change::Patch { path, edits, .. } => {
                let disk = self.root.join(path.vault_relative());
                let current = std::fs::read_to_string(&disk)
                    .map_err(|error| VaultError::Io(error.to_string()))?;
                let content = apply_text_edits(&current, edits)?;
                self.atomic_write(&path, content.as_bytes())?;
                revision = Some(Revision::of(content.as_bytes()));
                paths.push(path.vault_relative().display().to_string());
            }
            Change::Move { from, to } => {
                let source = self.root.join(from.vault_relative());
                let target = self.root.join(to.vault_relative());
                self.validate_destination_parent(&target, to.scope())?;
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|error| VaultError::Io(error.to_string()))?;
                }
                std::fs::rename(source, target)
                    .map_err(|error| VaultError::Io(error.to_string()))?;
                paths.extend([
                    from.vault_relative().display().to_string(),
                    to.vault_relative().display().to_string(),
                ]);
            }
            Change::Delete { path } => {
                let scope = match path.scope() {
                    Scope::Raw => CollectionScope::Sources,
                    Scope::Wiki => CollectionScope::Wiki,
                    Scope::All => unreachable!(),
                };
                self.engine()?
                    .delete_file(scope, &path.relative().to_string_lossy())?;
                paths.push(path.vault_relative().display().to_string());
            }
            Change::ImportRaw { files } => {
                for file in files {
                    self.atomic_write(&file.path, file.content.as_bytes())?;
                    paths.push(file.path.vault_relative().display().to_string());
                }
            }
            Change::IngestSource { source, details } => {
                let request = if let Some(details) = details {
                    IngestSourceRequest {
                        source_rel_path: source.vault_relative().to_string_lossy().into_owned(),
                        target_wiki_path: details.target_wiki_path,
                        raw_source_content: details.raw_source_content,
                        title: details.title,
                        one_line_summary: details.one_line_summary,
                        wiki_markdown_body: details.wiki_markdown_body,
                        tags: details.tags,
                        related_entities: details.related_entities,
                        related_concepts: details.related_concepts,
                    }
                } else {
                    let content = std::fs::read_to_string(self.root.join(source.vault_relative()))
                        .map_err(|error| VaultError::Io(error.to_string()))?;
                    let (title, summary) = source_title_summary(&content, &source);
                    IngestSourceRequest {
                        source_rel_path: source.vault_relative().to_string_lossy().into_owned(),
                        target_wiki_path: None,
                        raw_source_content: None,
                        title,
                        one_line_summary: summary,
                        wiki_markdown_body: content,
                        tags: Vec::new(),
                        related_entities: Vec::new(),
                        related_concepts: Vec::new(),
                    }
                };
                let result = self
                    .engine()?
                    .ingest_source_to_wiki(IngestSourceRequest { ..request })?;
                paths.extend([result.source_rel_path, result.wiki_rel_path]);
            }
            Change::FileSynthesis { path, details } => {
                let result =
                    self.engine()?
                        .file_query_synthesis(llm_wiki::FileSynthesisRequest {
                            rel_path: path.relative().to_string_lossy().into_owned(),
                            query: details.query,
                            title: details.title,
                            one_line_summary: details.one_line_summary,
                            markdown_body: details.markdown_body,
                            tags: details.tags,
                        })?;
                paths.push(result.rel_path);
            }
            Change::Reindex { force: _ } => {
                let stats = self.engine()?.sync_index()?;
                self.engine()?.rebuild_or_sync_index_md()?;
                self.publish_reindexed(stats);
            }
        }
        if !plan.paths.is_empty() && !specialized_indexing {
            self.engine()?.sync_index()?;
            self.engine()?.rebuild_or_sync_index_md()?;
            self.engine()?
                .append_log(LogAction::Custom, "Vault updated", &paths)?;
        }
        Ok((paths, revision))
    }
}
