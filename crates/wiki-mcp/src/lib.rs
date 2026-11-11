//! MCP tool host backed by the local wiki vault.

mod dispatch;
mod host;
mod paths;
mod result;
mod schema;
mod tree;
mod types;

pub use host::ToolHost;
pub(crate) use paths::{parse_scope, scoped_doc_path, scoped_doc_path_with_scope};
pub(crate) use tree::{limit_tree_depth, render_tree};
pub use types::{Access, CallCtx, Policy, ToolError, ToolSpec};

#[cfg(test)]
mod tests;
