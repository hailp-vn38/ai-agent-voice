use super::{
    Applied, Change, DocPath, Graph, GraphEdge, GraphNode, Hit, Origin, Page, ReadWindow,
    ReindexStats, Revision, Scope, TextEdit, TreeNode, VaultConfig, VaultError, VaultEvent,
    VaultStatus,
};
use llm_wiki::{
    CollectionScope, GetOptions, IngestSourceRequest, LogAction, SearchOptions, SyncStats,
    TreeOptions, TreeScope, WikiConfig, WikiEngine,
};
use std::collections::HashMap;
use std::io::Write;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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

#[derive(Default)]
struct ChangePlan {
    paths: Vec<PathBuf>,
}

struct FileSnapshot {
    relative: PathBuf,
    content: Option<Vec<u8>>,
}

struct DirectorySnapshot {
    relative: PathBuf,
    existed: bool,
}

struct SnapshotSet {
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

    pub fn subscribe(&self) -> broadcast::Receiver<VaultEvent> {
        self.events.subscribe()
    }

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

    pub fn notice_external(
        &self,
        paths: impl IntoIterator<Item = DocPath>,
    ) -> Result<bool, VaultError> {
        if !self.is_writable() {
            return Err(VaultError::ReadOnly);
        }
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| VaultError::Io("vault write lock poisoned".into()))?;
        let paths: Vec<PathBuf> = paths
            .into_iter()
            .map(|path| path.vault_relative())
            .collect();
        if paths.is_empty() {
            return Ok(false);
        }
        let mut external = Vec::new();
        let mut self_writes = self
            .self_writes
            .lock()
            .map_err(|_| VaultError::Io("vault self-write set poisoned".into()))?;
        for relative in paths {
            let current = match std::fs::read(self.root.join(&relative)) {
                Ok(content) => Some(Revision::of(&content)),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(VaultError::Io(error.to_string())),
            };
            match self_writes.get(&relative) {
                Some(written) if written == &current => continue,
                Some(_) => {
                    self_writes.remove(&relative);
                }
                None => {}
            }
            external.push(relative);
        }
        drop(self_writes);
        if external.is_empty() {
            return Ok(false);
        }
        self.engine.sync_index()?;
        self.engine.rebuild_or_sync_index_md()?;
        let _ = self.record_self_writes(&["wiki/index.md".into()]);
        let external: Vec<String> = external
            .iter()
            .map(|path| path.display().to_string())
            .collect();
        let _ = self.events.send(VaultEvent::External { paths: external });
        Ok(true)
    }

    fn record_self_writes(&self, paths: &[String]) -> Result<(), VaultError> {
        let mut self_writes = self
            .self_writes
            .lock()
            .map_err(|_| VaultError::Io("vault self-write set poisoned".into()))?;
        for path in paths {
            let relative = PathBuf::from(path);
            let revision = match std::fs::read(self.root.join(&relative)) {
                Ok(content) => Some(Revision::of(&content)),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(error) => return Err(VaultError::Io(error.to_string())),
            };
            self_writes.insert(relative, revision);
        }
        Ok(())
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
                if to.scope() == Scope::Wiki
                    && self
                        .engine
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
                    let wiki_rel = self.engine.resolve_wiki_path_for_source(&source_rel)?.0;
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
                self.engine
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
                    .engine
                    .ingest_source_to_wiki(IngestSourceRequest { ..request })?;
                paths.extend([result.source_rel_path, result.wiki_rel_path]);
            }
            Change::FileSynthesis { path, details } => {
                let result = self
                    .engine
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
                let stats = self.engine.sync_index()?;
                self.engine.rebuild_or_sync_index_md()?;
                self.publish_reindexed(stats);
            }
        }
        if !plan.paths.is_empty() && !specialized_indexing {
            self.engine.sync_index()?;
            self.engine.rebuild_or_sync_index_md()?;
            self.engine
                .append_log(LogAction::Custom, "Vault updated", &paths)?;
        }
        Ok((paths, revision))
    }

    fn publish_reindexed(&self, stats: SyncStats) {
        let status = self.engine.status();
        if let Ok(status) = status {
            let _ = self.events.send(VaultEvent::Reindexed {
                stats: ReindexStats {
                    raw_documents: status.sources_count,
                    wiki_pages: status.wiki_count,
                    total_chunks: stats.total_chunks,
                },
            });
        }
    }

    fn validate_target(&self, path: &DocPath) -> Result<(), VaultError> {
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

    fn validate_destination_parent(&self, target: &Path, scope: Scope) -> Result<(), VaultError> {
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

    fn atomic_write(&self, path: &DocPath, content: &[u8]) -> Result<(), VaultError> {
        let target = self.root.join(path.vault_relative());
        self.validate_target(path)?;
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|error| VaultError::Io(error.to_string()))?;
        }
        self.validate_destination_parent(&target, path.scope())?;
        write_atomic(&target, content)
    }

    fn snapshot_paths(&self, paths: &[PathBuf]) -> Result<SnapshotSet, VaultError> {
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

    fn restore_snapshots(&self, snapshots: SnapshotSet) -> Result<(), VaultError> {
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

    pub fn tree(&self, scope: Scope) -> Result<TreeNode, VaultError> {
        let engine_scope = match scope {
            Scope::Raw => TreeScope::Sources,
            Scope::Wiki => TreeScope::Wiki,
            Scope::All => TreeScope::All,
        };
        let tree = self.engine.get_tree(TreeOptions::for_scope(engine_scope))?;
        fn convert(node: llm_wiki::FileTreeNode) -> TreeNode {
            TreeNode {
                name: node.name,
                path: node.vault_rel_path,
                is_directory: node.node_type == llm_wiki::NodeType::Directory,
                children: node.children.into_iter().map(convert).collect(),
            }
        }
        Ok(convert(tree))
    }

    pub fn read(&self, path: &DocPath, window: ReadWindow) -> Result<Page, VaultError> {
        let full_path = self.root.join(path.vault_relative());
        let canonical = full_path.canonicalize().map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                VaultError::NotFound(path.vault_relative().display().to_string())
            } else {
                VaultError::Io(error.to_string())
            }
        })?;
        let scope_root = self.root.join(match path.scope() {
            Scope::Raw => "sources",
            Scope::Wiki => "wiki",
            Scope::All => unreachable!(),
        });
        let scope_root = scope_root
            .canonicalize()
            .map_err(|error| VaultError::Io(error.to_string()))?;
        if !scope_root.starts_with(&self.root) || !canonical.starts_with(scope_root) {
            return Err(VaultError::InvalidPath(
                path.vault_relative().display().to_string(),
            ));
        }

        let result = self.engine.get_document(
            &path.vault_relative().to_string_lossy(),
            GetOptions {
                from_line: window.from_line,
                max_lines: window.max_lines,
                ..Default::default()
            },
        )?;
        let revision = Revision::of(
            &std::fs::read(&full_path).map_err(|error| VaultError::Io(error.to_string()))?,
        );
        Ok(Page {
            path: result.info.rel_path,
            title: result.info.title,
            content: result.content,
            revision,
        })
    }

    pub fn search(&self, query: &str) -> Result<Vec<Hit>, VaultError> {
        let results = self.engine.search_lex(
            query,
            SearchOptions {
                collections: vec![CollectionScope::Sources, CollectionScope::Wiki],
                limit: 50,
                ..Default::default()
            },
        )?;
        Ok(results
            .into_iter()
            .map(|item| Hit {
                path: item.rel_path,
                title: item.title,
                score: item.score,
                line: item.match_line,
                snippet: item.snippet,
            })
            .collect())
    }

    pub fn graph(&self, focus: Option<&DocPath>) -> Result<Graph, VaultError> {
        let graph = self.engine.get_graph(None)?;
        let mut graph = Graph {
            nodes: graph
                .nodes
                .into_iter()
                .map(|node| GraphNode {
                    path: node.rel_path,
                    label: node.label,
                    title: node.title,
                    inbound: node.inbound_degree,
                    outbound: node.outbound_degree,
                })
                .collect(),
            edges: graph
                .edges
                .into_iter()
                .map(|edge| GraphEdge {
                    source: edge.source_rel_path,
                    target: edge.resolved_rel_path,
                    raw_target: edge.raw_target,
                    line: edge.line_number,
                })
                .collect(),
            unresolved: graph.unresolved_targets,
        };
        if let Some(focus) = focus {
            let focus_path = focus.vault_relative().to_string_lossy().into_owned();
            let mut included = std::collections::HashSet::from([focus_path.clone()]);
            graph.edges.retain(|edge| {
                let adjacent =
                    edge.source == focus_path || edge.target.as_deref() == Some(&focus_path);
                if adjacent {
                    included.insert(edge.source.clone());
                    if let Some(target) = &edge.target {
                        included.insert(target.clone());
                    }
                }
                adjacent
            });
            graph.nodes.retain(|node| included.contains(&node.path));
            graph.unresolved = graph
                .edges
                .iter()
                .filter(|edge| edge.target.is_none())
                .map(|edge| edge.raw_target.clone())
                .collect();
        }
        Ok(graph)
    }

    pub fn status(&self) -> Result<VaultStatus, VaultError> {
        let status = self.engine.status()?;
        Ok(VaultStatus {
            raw_documents: status.sources_count,
            wiki_pages: status.wiki_count,
            graph_links: status.total_graph_links,
            resolved_links: status.resolved_graph_links,
            unresolved_links: status.unresolved_graph_links,
        })
    }

    /// Run one of the library's read-only Wiki MCP tools without exposing its engine types.
    /// Tool names are checked against the upstream read-only allowlist before dispatch.
    pub fn read_tool(
        &self,
        name: &str,
        arguments: serde_json::Value,
        max_result_chars: usize,
    ) -> Result<serde_json::Value, VaultError> {
        if !llm_wiki::is_read_only_wiki_tool(name) {
            return Err(VaultError::InvalidPath(format!(
                "not a read-only tool: {name}"
            )));
        }
        let budget = llm_wiki::AdaptiveReadBudget {
            default_lines: 160,
            max_window_chars: max_result_chars.saturating_sub(1024).max(1024),
            compact_index_max_chars: max_result_chars.saturating_sub(1024).max(1024),
        };
        let outcome = self
            .engine
            .execute_llm_tool_with_budget(name, arguments, budget)?;
        Ok(serde_json::json!({
            "tool": name,
            "content": outcome.content,
            "metadata": outcome.metadata,
        }))
    }

    /// Return the library's canonical document metadata for a file in this vault.
    pub fn document_metadata(&self, path: &DocPath) -> Result<serde_json::Value, VaultError> {
        let document = self.engine.get_document(
            &path.vault_relative().to_string_lossy(),
            GetOptions::default(),
        )?;
        serde_json::to_value(document.info).map_err(|error| VaultError::Index(error.to_string()))
    }
}
