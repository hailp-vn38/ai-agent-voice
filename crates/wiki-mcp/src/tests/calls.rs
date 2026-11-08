#[tokio::test]
async fn calls_read_vault_status_and_apply_writes_through_vault() {
    let (_dir, host) = host(true);
    let status = host
        .call(context(Access::ReadOnly), "wiki_status", json!({}))
        .await
        .unwrap();
    assert_eq!(status["tool"], "wiki_status");
    let write = host
        .call(
            context(Access::ReadWrite),
            "wiki_write_file",
            json!({"scope":"wiki","rel_path":"concepts/mcp.md","content":"# MCP\n\nShared host."}),
        )
        .await
        .unwrap();
    assert!(
        write["content"]
            .as_str()
            .unwrap()
            .contains("concepts/mcp.md")
    );
    let query = host
        .call(
            context(Access::ReadOnly),
            "wiki_query",
            json!({"query":"Shared host"}),
        )
        .await
        .unwrap();
    assert!(query["content"].as_str().unwrap().contains("mcp.md"));
}
use super::{context, host};
use crate::Access;
use serde_json::json;
