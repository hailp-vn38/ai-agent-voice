use rmcp::{ServiceExt, model::CallToolRequestParams, transport::child_process::TokioChildProcess};
use tempfile::TempDir;
use tokio::process::Command;
use wiki_core::{Vault, VaultConfig};

fn command(root: &TempDir) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_wiki-mcp"));
    command.arg("--vault").arg(root.path()).arg("--allow-write");
    command
}

#[tokio::test]
async fn direct_stdio_initializes_lists_tools_and_calls_status() {
    let root = TempDir::new().unwrap();
    let transport = TokioChildProcess::new(command(&root)).unwrap();
    let client = ().serve(transport).await.unwrap();

    let tools = client.list_all_tools().await.unwrap();
    assert!(tools.iter().any(|tool| tool.name == "wiki_status"));
    assert!(tools.iter().any(|tool| tool.name == "wiki_write_file"));

    let result = client
        .call_tool(CallToolRequestParams::new("wiki_status"))
        .await
        .unwrap();
    assert!(!result.is_error.unwrap_or(false));
    let status = result.structured_content.unwrap();
    assert_eq!(status["tool"], "wiki_status");
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn direct_stdio_keeps_reads_available_and_reports_read_only_writes() {
    let root = TempDir::new().unwrap();
    let writer = Vault::open(VaultConfig::new(root.path())).unwrap();
    assert!(writer.is_writable());

    let transport = TokioChildProcess::new(command(&root)).unwrap();
    let client = ().serve(transport).await.unwrap();
    let status = client
        .call_tool(CallToolRequestParams::new("wiki_status"))
        .await
        .unwrap();
    assert!(!status.is_error.unwrap_or(false));

    let write = client
        .call_tool(
            CallToolRequestParams::new("wiki_write_file").with_arguments(
                serde_json::json!({
                    "scope": "wiki",
                    "rel_path": "notes/blocked.md",
                    "content": "# Blocked",
                })
                .as_object()
                .unwrap()
                .clone(),
            ),
        )
        .await
        .unwrap();
    assert!(write.is_error.unwrap_or(false));
    assert!(write.content.iter().any(|block| {
        block
            .as_text()
            .is_some_and(|text| text.text.contains("read-only"))
    }));
    client.cancel().await.unwrap();
}
