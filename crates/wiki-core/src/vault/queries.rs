//! Read-only document, search, graph, status, and Wiki tool access.

use super::Vault;
use crate::{
    DocPath, Graph, GraphEdge, GraphNode, Hit, Page, ReadWindow, Revision, Scope, TreeNode,
    VaultError, VaultStatus,
};
use llm_wiki::{CollectionScope, GetOptions, SearchOptions, TreeOptions, TreeScope};

impl Vault {
    pub fn tree(&self, scope: Scope) -> Result<TreeNode, VaultError> {
        let engine_scope = match scope {
            Scope::Raw => TreeScope::Sources,
            Scope::Wiki => TreeScope::Wiki,
            Scope::All => TreeScope::All,
        };
        let tree = self
            .engine()?
            .get_tree(TreeOptions::for_scope(engine_scope))?;
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

        let result = self.engine()?.get_document(
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
        let results = self.engine()?.search_lex(
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
        let graph = self.engine()?.get_graph(None)?;
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
        let status = self.engine()?.status()?;
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
            .engine()?
            .execute_llm_tool_with_budget(name, arguments, budget)?;
        Ok(serde_json::json!({
            "tool": name,
            "content": outcome.content,
            "metadata": outcome.metadata,
        }))
    }

    /// Return the library's canonical document metadata for a file in this vault.
    pub fn document_metadata(&self, path: &DocPath) -> Result<serde_json::Value, VaultError> {
        let document = self.engine()?.get_document(
            &path.vault_relative().to_string_lossy(),
            GetOptions::default(),
        )?;
        serde_json::to_value(document.info).map_err(|error| VaultError::Index(error.to_string()))
    }
}
