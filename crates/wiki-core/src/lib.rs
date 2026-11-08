use llm_wiki::{
    CollectionScope, GetOptions, IngestSourceRequest, LogAction, SearchOptions, SyncStats,
    TreeOptions, TreeScope, WikiConfig, WikiEngine, WikiError,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use thiserror::Error;
use tokio::sync::broadcast;

#[derive(Debug, Clone)]
pub struct VaultConfig {
    pub root: PathBuf,
}

impl VaultConfig {
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    Raw,
    Wiki,
    All,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct DocPath {
    scope: DocumentScope,
    relative: PathBuf,
}

impl<'de> Deserialize<'de> for DocPath {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct EncodedPath {
            scope: DocumentScope,
            relative: PathBuf,
        }

        let encoded = EncodedPath::deserialize(deserializer)?;
        let scope = match encoded.scope {
            DocumentScope::Raw => Scope::Raw,
            DocumentScope::Wiki => Scope::Wiki,
        };
        DocPath::parse(scope, encoded.relative).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DocumentScope {
    Raw,
    Wiki,
}

impl DocPath {
    pub fn parse(scope: Scope, path: impl AsRef<Path>) -> Result<Self, VaultError> {
        let path = path.as_ref();
        if matches!(scope, Scope::All)
            || path.as_os_str().is_empty()
            || path.to_string_lossy().contains('\\')
            || path.to_string_lossy().contains(':')
            || path.to_string_lossy().contains('\0')
            || path
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
            || path.extension().is_none_or(|extension| extension != "md")
        {
            return Err(VaultError::InvalidPath(path.to_string_lossy().into_owned()));
        }

        Ok(Self {
            scope: match scope {
                Scope::Raw => DocumentScope::Raw,
                Scope::Wiki => DocumentScope::Wiki,
                Scope::All => unreachable!(),
            },
            relative: path.to_path_buf(),
        })
    }

    pub fn scope(&self) -> Scope {
        match self.scope {
            DocumentScope::Raw => Scope::Raw,
            DocumentScope::Wiki => Scope::Wiki,
        }
    }

    pub fn relative(&self) -> &Path {
        &self.relative
    }

    pub fn vault_relative(&self) -> PathBuf {
        let root = match self.scope {
            DocumentScope::Raw => "sources",
            DocumentScope::Wiki => "wiki",
        };
        Path::new(root).join(&self.relative)
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct ReadWindow {
    pub from_line: Option<usize>,
    pub max_lines: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TreeNode {
    pub name: String,
    pub path: String,
    pub is_directory: bool,
    pub children: Vec<TreeNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Page {
    pub path: String,
    pub title: String,
    pub content: String,
    pub revision: Revision,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hit {
    pub path: String,
    pub title: String,
    pub score: f32,
    pub line: usize,
    pub snippet: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Graph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    pub unresolved: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphNode {
    pub path: String,
    pub label: String,
    pub title: String,
    pub inbound: usize,
    pub outbound: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphEdge {
    pub source: String,
    pub target: Option<String>,
    pub raw_target: String,
    pub line: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultStatus {
    pub raw_documents: usize,
    pub wiki_pages: usize,
    pub graph_links: usize,
    pub resolved_links: usize,
    pub unresolved_links: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Revision(pub String);

impl Revision {
    fn of(content: &[u8]) -> Self {
        Self(
            Sha256::digest(content)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    Ui,
    Mcp,
    Server,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextEdit {
    /// Zero-based line at which this edit starts.
    pub start_line: usize,
    pub delete_lines: usize,
    pub insert: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawFile {
    pub path: DocPath,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Change {
    Write {
        path: DocPath,
        content: String,
        expect: Option<Revision>,
    },
    Patch {
        path: DocPath,
        edits: Vec<TextEdit>,
        expect: Option<Revision>,
    },
    Move {
        from: DocPath,
        to: DocPath,
    },
    Delete {
        path: DocPath,
    },
    ImportRaw {
        files: Vec<RawFile>,
    },
    IngestSource {
        source: DocPath,
    },
    Reindex {
        force: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Applied {
    pub paths: Vec<String>,
    pub revision: Option<Revision>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum VaultEvent {
    Changed { paths: Vec<String>, origin: Origin },
    Reindexed { stats: ReindexStats },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReindexStats {
    pub raw_documents: usize,
    pub wiki_pages: usize,
    pub total_chunks: usize,
}

#[derive(Debug, Error)]
pub enum VaultError {
    #[error("Document not found: {0}")]
    NotFound(String),
    #[error("Invalid document path: {0}")]
    InvalidPath(String),
    #[error("Document changed since it was read")]
    Conflict { current: Revision },
    #[error("This process does not hold the vault write lock")]
    ReadOnly,
    #[error("I/O error: {0}")]
    Io(String),
    #[error("Knowledge index error: {0}")]
    Index(String),
}

impl From<WikiError> for VaultError {
    fn from(error: WikiError) -> Self {
        match error {
            WikiError::DocumentNotFound { target, .. } => Self::NotFound(target),
            WikiError::Io(error) => Self::Io(error.to_string()),
            WikiError::PathTraversal(path) | WikiError::InvalidFileName(path) => {
                Self::InvalidPath(path)
            }
            other => Self::Index(other.to_string()),
        }
    }
}

pub struct Vault {
    root: PathBuf,
    engine: WikiEngine,
    write_lock: Mutex<()>,
    events: broadcast::Sender<VaultEvent>,
}

#[derive(Default)]
struct ChangePlan {
    paths: Vec<PathBuf>,
}

struct FileSnapshot {
    relative: PathBuf,
    content: Option<Vec<u8>>,
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
        let engine = WikiEngine::open_with_config(WikiConfig::new(&root))?;
        let (events, _) = broadcast::channel(64);
        Ok(Self {
            root,
            engine,
            write_lock: Mutex::new(()),
            events,
        })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<VaultEvent> {
        self.events.subscribe()
    }

    pub fn apply(&self, change: Change) -> Result<Applied, VaultError> {
        self.apply_from(Origin::Server, change)
    }

    pub fn apply_from(&self, origin: Origin, change: Change) -> Result<Applied, VaultError> {
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
            Change::IngestSource { source } => {
                if source.scope() != Scope::Raw {
                    return Err(VaultError::InvalidPath(
                        "IngestSource requires a raw path".into(),
                    ));
                }
                self.validate_target(source)?;
                let source_path = self.root.join(source.vault_relative());
                if !source_path.is_file() {
                    return Err(VaultError::NotFound(
                        source.vault_relative().display().to_string(),
                    ));
                }
                plan.paths.push(source.vault_relative());
                let source_rel = source.vault_relative().to_string_lossy().into_owned();
                let wiki_rel = self.engine.resolve_wiki_path_for_source(&source_rel)?.0;
                plan.paths.push(PathBuf::from(wiki_rel));
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
                let target = self.root.join(path.vault_relative());
                if target.exists() {
                    std::fs::remove_file(target)
                        .map_err(|error| VaultError::Io(error.to_string()))?;
                }
                paths.push(path.vault_relative().display().to_string());
            }
            Change::ImportRaw { files } => {
                for file in files {
                    self.atomic_write(&file.path, file.content.as_bytes())?;
                    paths.push(file.path.vault_relative().display().to_string());
                }
            }
            Change::IngestSource { source } => {
                let content = std::fs::read_to_string(self.root.join(source.vault_relative()))
                    .map_err(|error| VaultError::Io(error.to_string()))?;
                let (title, summary) = source_title_summary(&content, &source);
                let result = self.engine.ingest_source_to_wiki(IngestSourceRequest {
                    source_rel_path: source.vault_relative().to_string_lossy().into_owned(),
                    target_wiki_path: None,
                    raw_source_content: None,
                    title,
                    one_line_summary: summary,
                    wiki_markdown_body: content,
                    tags: Vec::new(),
                    related_entities: Vec::new(),
                    related_concepts: Vec::new(),
                })?;
                paths.extend([result.source_rel_path, result.wiki_rel_path]);
            }
            Change::Reindex { force: _ } => {
                let stats = self.engine.sync_index()?;
                self.engine.rebuild_or_sync_index_md()?;
                self.publish_reindexed(stats);
            }
        }
        if !matches!(plan.paths.as_slice(), []) {
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

    fn snapshot_paths(&self, paths: &[PathBuf]) -> Result<Vec<FileSnapshot>, VaultError> {
        paths
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
            .collect()
    }

    fn restore_snapshots(&self, snapshots: Vec<FileSnapshot>) -> Result<(), VaultError> {
        for snapshot in snapshots {
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
}

#[cfg(test)]
mod tests {
    use super::{Change, DocPath, Origin, RawFile, Revision, Scope, TextEdit, Vault, VaultConfig};
    use llm_wiki::{CollectionScope, WikiEngine};
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn opening_an_empty_vault_creates_the_standard_tree() {
        let temp = tempdir().unwrap();
        let vault = Vault::open(VaultConfig::new(temp.path())).unwrap();

        assert!(temp.path().join("sources").is_dir());
        assert!(temp.path().join("wiki").is_dir());
        assert_eq!(vault.tree(Scope::All).unwrap().name, ".");
    }

    #[test]
    fn opening_existing_documents_supports_tree_read_search_and_graph() {
        let temp = tempdir().unwrap();
        let engine = WikiEngine::open(temp.path()).unwrap();
        engine
            .write_file(
                CollectionScope::Wiki,
                "concepts/tri-thuc.md",
                "# Tri thức\n\nHệ thống quản lý tri thức cá nhân.",
            )
            .unwrap();
        engine
            .write_file(
                CollectionScope::Wiki,
                "concepts/agent.md",
                "# Agent\n\nXem [[tri-thuc]].",
            )
            .unwrap();
        drop(engine);

        let vault = Vault::open(VaultConfig::new(temp.path())).unwrap();
        let tree = vault.tree(Scope::Wiki).unwrap();
        assert_eq!(tree.name, "wiki");
        assert!(tree.children.iter().any(|node| node.name == "concepts"));

        let path = DocPath::parse(Scope::Wiki, "concepts/tri-thuc.md").unwrap();
        let page = vault.read(&path, Default::default()).unwrap();
        assert!(page.content.contains("Hệ thống quản lý tri thức"));

        let hits = vault.search("tri thuc").unwrap();
        assert!(
            hits.iter()
                .any(|hit| hit.path == "wiki/concepts/tri-thuc.md")
        );

        let graph = vault.graph(None).unwrap();
        assert!(
            graph
                .nodes
                .iter()
                .any(|node| node.path == "wiki/concepts/tri-thuc.md")
        );
        assert!(
            graph
                .edges
                .iter()
                .any(|edge| edge.source == "wiki/concepts/agent.md")
        );
        let focused = vault.graph(Some(&path)).unwrap();
        assert_eq!(focused.nodes.len(), 2);
        assert_eq!(focused.edges.len(), 1);
    }

    #[test]
    fn document_paths_reject_traversal_absolute_paths_and_escaping_symlinks() {
        let temp = tempdir().unwrap();
        let outside = tempdir().unwrap();
        fs::write(outside.path().join("secret.md"), "private").unwrap();
        let vault_root = temp.path().join("vault");
        let vault = Vault::open(VaultConfig::new(&vault_root)).unwrap();
        std::os::unix::fs::symlink(outside.path(), vault_root.join("wiki/external")).unwrap();

        assert!(DocPath::parse(Scope::Wiki, "../secret.md").is_err());
        assert!(DocPath::parse(Scope::Wiki, "/etc/passwd").is_err());
        let escaping = DocPath::parse(Scope::Wiki, "external/secret.md").unwrap();
        assert!(vault.read(&escaping, Default::default()).is_err());
        assert!(
            serde_json::from_str::<DocPath>(r#"{"scope":"wiki","relative":"../secret.md"}"#)
                .is_err()
        );
    }

    #[test]
    fn writing_a_page_updates_search_and_publishes_one_change() {
        let temp = tempdir().unwrap();
        let vault = Vault::open(VaultConfig::new(temp.path())).unwrap();
        let mut events = vault.subscribe();
        let path = DocPath::parse(Scope::Wiki, "concepts/locking.md").unwrap();

        vault
            .apply_from(
                Origin::Ui,
                Change::Write {
                    path: path.clone(),
                    content: "# Locking\n\nSingle writer lease behavior.".into(),
                    expect: None,
                },
            )
            .unwrap();

        assert!(
            vault
                .read(&path, Default::default())
                .unwrap()
                .content
                .contains("Single writer")
        );
        assert!(
            vault
                .search("single writer")
                .unwrap()
                .iter()
                .any(|hit| hit.path == "wiki/concepts/locking.md")
        );
        let event = events.try_recv().unwrap();
        assert!(matches!(
            event,
            super::VaultEvent::Changed {
                origin: Origin::Ui,
                ..
            }
        ));
    }

    #[test]
    fn writing_with_a_stale_revision_leaves_the_page_unchanged() {
        let temp = tempdir().unwrap();
        let vault = Vault::open(VaultConfig::new(temp.path())).unwrap();
        let path = DocPath::parse(Scope::Wiki, "concepts/revision.md").unwrap();
        vault
            .apply(Change::Write {
                path: path.clone(),
                content: "# Current\n\nKeep this content.".into(),
                expect: None,
            })
            .unwrap();

        let result = vault.apply(Change::Write {
            path: path.clone(),
            content: "# Stale\n\nMust not overwrite.".into(),
            expect: Some(Revision("stale-revision".into())),
        });

        assert!(matches!(result, Err(super::VaultError::Conflict { .. })));
        assert!(
            vault
                .read(&path, Default::default())
                .unwrap()
                .content
                .contains("Keep this content")
        );
    }

    #[test]
    fn patch_move_delete_import_ingest_and_reindex_work_through_apply() {
        let temp = tempdir().unwrap();
        let vault = Vault::open(VaultConfig::new(temp.path())).unwrap();
        let source = DocPath::parse(Scope::Raw, "notes/first.md").unwrap();
        let page = DocPath::parse(Scope::Wiki, "concepts/first.md").unwrap();
        let moved = DocPath::parse(Scope::Wiki, "concepts/renamed.md").unwrap();

        vault
            .apply(Change::Write {
                path: page.clone(),
                content: "# First\n\nOld body.".into(),
                expect: None,
            })
            .unwrap();
        let revision = vault.read(&page, Default::default()).unwrap().revision;
        vault
            .apply(Change::Patch {
                path: page.clone(),
                edits: vec![TextEdit {
                    start_line: 2,
                    delete_lines: 1,
                    insert: "New body.".into(),
                }],
                expect: Some(revision),
            })
            .unwrap();
        assert!(
            vault
                .read(&page, Default::default())
                .unwrap()
                .content
                .contains("New body")
        );

        vault
            .apply(Change::Move {
                from: page.clone(),
                to: moved.clone(),
            })
            .unwrap();
        assert!(vault.read(&moved, Default::default()).is_ok());
        assert!(matches!(
            vault.read(&page, Default::default()),
            Err(super::VaultError::NotFound(_))
        ));

        vault
            .apply(Change::ImportRaw {
                files: vec![RawFile {
                    path: source.clone(),
                    content: "# Source\n\nImported raw note for the wiki.".into(),
                }],
            })
            .unwrap();
        vault
            .apply(Change::IngestSource {
                source: source.clone(),
            })
            .unwrap();
        assert!(
            vault
                .search("Imported raw note")
                .unwrap()
                .iter()
                .any(|hit| hit.path.starts_with("wiki/"))
        );

        vault.apply(Change::Reindex { force: true }).unwrap();
        vault
            .apply(Change::Delete {
                path: moved.clone(),
            })
            .unwrap();
        assert!(matches!(
            vault.read(&moved, Default::default()),
            Err(super::VaultError::NotFound(_))
        ));
    }

    #[test]
    fn concurrent_writes_with_the_same_revision_do_not_lose_an_update() {
        let temp = tempdir().unwrap();
        let vault = std::sync::Arc::new(Vault::open(VaultConfig::new(temp.path())).unwrap());
        let path = DocPath::parse(Scope::Wiki, "concepts/shared.md").unwrap();
        vault
            .apply(Change::Write {
                path: path.clone(),
                content: "# Original".into(),
                expect: None,
            })
            .unwrap();
        let revision = vault.read(&path, Default::default()).unwrap().revision;
        let mut workers = Vec::new();
        for body in ["# First", "# Second"] {
            let vault = vault.clone();
            let path = path.clone();
            let revision = revision.clone();
            workers.push(std::thread::spawn(move || {
                vault.apply(Change::Write {
                    path,
                    content: body.into(),
                    expect: Some(revision),
                })
            }));
        }
        let results: Vec<_> = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect();
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, Err(super::VaultError::Conflict { .. })))
                .count(),
            1
        );
    }

    #[test]
    fn failed_write_restores_files_and_does_not_publish_change() {
        let temp = tempdir().unwrap();
        let vault = Vault::open(VaultConfig::new(temp.path())).unwrap();
        let blocker = DocPath::parse(Scope::Wiki, "blocked.md").unwrap();
        vault
            .apply(Change::Write {
                path: blocker,
                content: "not a directory".into(),
                expect: None,
            })
            .unwrap();
        let mut events = vault.subscribe();
        let nested = DocPath::parse(Scope::Wiki, "blocked.md/child.md").unwrap();

        assert!(
            vault
                .apply(Change::Write {
                    path: nested,
                    content: "# Never committed".into(),
                    expect: None,
                })
                .is_err()
        );
        assert!(events.try_recv().is_err());
        assert!(!temp.path().join("wiki/blocked.md/child.md").exists());
        assert_eq!(
            std::fs::read_dir(temp.path().join("wiki"))
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| entry.file_name().to_string_lossy().contains("tmp"))
                .count(),
            0
        );

        let first = DocPath::parse(Scope::Raw, "batch/first.md").unwrap();
        let second = DocPath::parse(Scope::Raw, "blocked.md/second.md").unwrap();
        fs::write(temp.path().join("sources/blocked.md"), "not a directory").unwrap();
        assert!(
            vault
                .apply(Change::ImportRaw {
                    files: vec![
                        RawFile {
                            path: first.clone(),
                            content: "# First batch file".into(),
                        },
                        RawFile {
                            path: second,
                            content: "# Second batch file".into(),
                        },
                    ],
                })
                .is_err()
        );
        assert!(!temp.path().join(first.vault_relative()).exists());
        assert!(vault.search("First batch file").unwrap().is_empty());
        assert!(events.try_recv().is_err());
    }

    #[test]
    fn moving_a_page_with_real_incoming_wiki_links_is_rejected() {
        let temp = tempdir().unwrap();
        let vault = Vault::open(VaultConfig::new(temp.path())).unwrap();
        let page = DocPath::parse(Scope::Wiki, "concepts/linked.md").unwrap();
        let referrer = DocPath::parse(Scope::Wiki, "concepts/referrer.md").unwrap();
        let moved = DocPath::parse(Scope::Wiki, "concepts/renamed.md").unwrap();
        for (path, content) in [
            (page.clone(), "# Linked page"),
            (referrer, "# Referrer\n\nSee [[linked]]."),
        ] {
            vault
                .apply(Change::Write {
                    path,
                    content: content.into(),
                    expect: None,
                })
                .unwrap();
        }

        assert!(matches!(
            vault.apply(Change::Move {
                from: page,
                to: moved
            }),
            Err(super::VaultError::InvalidPath(_))
        ));
    }
}
