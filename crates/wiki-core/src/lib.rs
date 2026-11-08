use llm_wiki::{
    CollectionScope, GetOptions, SearchOptions, TreeOptions, TreeScope, WikiConfig, WikiEngine,
    WikiError,
};
use serde::{Deserialize, Serialize};
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

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DocPath {
    scope: DocumentScope,
    relative: PathBuf,
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

#[derive(Debug, Error)]
pub enum VaultError {
    #[error("Document not found: {0}")]
    NotFound(String),
    #[error("Invalid document path: {0}")]
    InvalidPath(String),
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
}

impl Vault {
    pub fn open(config: VaultConfig) -> Result<Self, VaultError> {
        std::fs::create_dir_all(&config.root).map_err(|error| VaultError::Io(error.to_string()))?;
        let root = config
            .root
            .canonicalize()
            .map_err(|error| VaultError::Io(error.to_string()))?;
        let engine = WikiEngine::open_with_config(WikiConfig::new(&root))?;
        Ok(Self { root, engine })
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
        if !canonical.starts_with(scope_root) {
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
        Ok(Page {
            path: result.info.rel_path,
            title: result.info.title,
            content: result.content,
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
    use super::{DocPath, Scope, Vault, VaultConfig};
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
    }
}
