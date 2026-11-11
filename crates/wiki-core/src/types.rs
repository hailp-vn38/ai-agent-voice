use llm_wiki::WikiError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Component, Path, PathBuf};
use thiserror::Error;

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
    pub(crate) fn of(content: &[u8]) -> Self {
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
pub struct IngestSourceDetails {
    pub target_wiki_path: Option<String>,
    pub raw_source_content: Option<String>,
    pub title: String,
    pub one_line_summary: String,
    pub wiki_markdown_body: String,
    pub tags: Vec<String>,
    pub related_entities: Vec<String>,
    pub related_concepts: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SynthesisDetails {
    pub query: String,
    pub title: String,
    pub one_line_summary: String,
    pub markdown_body: String,
    pub tags: Vec<String>,
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
        details: Option<IngestSourceDetails>,
    },
    FileSynthesis {
        path: DocPath,
        details: SynthesisDetails,
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
#[serde(tag = "type", rename_all = "snake_case")]
pub enum VaultEvent {
    Changed { paths: Vec<String>, origin: Origin },
    External { paths: Vec<String> },
    Reindexed { stats: ReindexStats },
    Lock { writable: bool },
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
