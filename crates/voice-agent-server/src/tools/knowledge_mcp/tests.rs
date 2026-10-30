use super::*;
use crate::tools::external_mcp::registry::{ExternalToolCatalog, validate_external_tool_schema};

struct TempVaultDir(PathBuf);

impl Drop for TempVaultDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn test_engine() -> (TempVaultDir, Arc<WikiEngine>) {
    let path = std::env::temp_dir().join(format!(
        "voice-agent-knowledge-mcp-test-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&path).expect("create temp vault dir");
    let engine = WikiEngine::open(&path).expect("open wiki engine");
    (TempVaultDir(path), Arc::new(engine))
}

fn test_ctx(access: KnowledgeMcpAccess) -> KnowledgeToolContext {
    KnowledgeToolContext {
        access,
        cancellation: CancellationToken::new(),
        deadline: Instant::now() + Duration::from_secs(10),
    }
}

#[test]
fn all_published_schemas_pass_external_mcp_schema_validation() {
    let (_dir, engine) = test_engine();
    let expected_ro = engine
        .llm_tool_definitions()
        .iter()
        .filter(|s| is_read_only_wiki_tool(&s.name))
        .count();
    let expected_rw = engine.llm_tool_definitions().len() + 1;
    let mcp = KnowledgeMcp::new(engine, None, true, 12_000);

    let ro_tools = mcp.list_tools(&test_ctx(KnowledgeMcpAccess::ReadOnly));
    assert_eq!(ro_tools.len(), expected_ro);
    for tool in &ro_tools {
        validate_external_tool_schema(&tool.parameters)
            .unwrap_or_else(|e| panic!("RO tool {} failed schema validation: {e}", tool.name));
    }

    let rw_tools = mcp.list_tools(&test_ctx(KnowledgeMcpAccess::ReadWrite));
    assert_eq!(rw_tools.len(), expected_rw);
    assert!(rw_tools.iter().any(|t| t.name == "add_to_sources"));
    for tool in &rw_tools {
        validate_external_tool_schema(&tool.parameters)
            .unwrap_or_else(|e| panic!("RW tool {} failed schema validation: {e}", tool.name));
    }

    let discovered: Vec<(String, String, Value)> = rw_tools
        .iter()
        .map(|t| (t.name.clone(), t.description.clone(), t.parameters.clone()))
        .collect();
    let catalog = ExternalToolCatalog::publish("external.knowledge", discovered)
        .expect("catalog should be accepted by ExternalToolCatalog");
    assert_eq!(catalog.tools().len(), expected_rw);
    assert_eq!(catalog.dropped(), 0);
}

#[tokio::test]
async fn read_only_caller_cannot_invoke_write_tools() {
    let (_dir, engine) = test_engine();
    let mcp = KnowledgeMcp::new(engine, None, true, 12_000);
    let ro_ctx = test_ctx(KnowledgeMcpAccess::ReadOnly);

    let status = mcp
        .call_tool(&ro_ctx, "wiki_status", json!({}))
        .await
        .expect("wiki_status should succeed for ReadOnly");
    assert_eq!(
        status.get("tool").and_then(Value::as_str),
        Some("wiki_status")
    );

    let write_err = mcp
        .call_tool(
            &ro_ctx,
            "wiki_write_file",
            json!({
                "scope": "wiki",
                "rel_path": "entities/rust.md",
                "content": "# Rust\n\nSystems programming language.\n"
            }),
        )
        .await
        .expect_err("ReadOnly caller must be denied wiki_write_file");
    assert_eq!(write_err, KnowledgeToolError::Denied);
}

#[tokio::test]
async fn read_write_caller_can_write_query_and_stage_sources() {
    let (_dir, engine) = test_engine();
    let notify = Arc::new(Notify::new());
    let mcp = KnowledgeMcp::new(engine, Some(Arc::clone(&notify)), true, 12_000);
    let rw_ctx = test_ctx(KnowledgeMcpAccess::ReadWrite);

    let write_res = mcp
        .call_tool(
            &rw_ctx,
            "wiki_write_file",
            json!({
                "scope": "wiki",
                "rel_path": "entities/voice-agent.md",
                "content": "---\ntitle: Voice Agent\ntags:\n  - voice\n---\n\n# Voice Agent\n\nConnects to [[knowledge-mcp]] over Streamable HTTP.\n"
            }),
        )
        .await
        .expect("wiki_write_file should succeed for ReadWrite");
    assert!(
        write_res
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .contains("entities/voice-agent.md")
    );

    let get_res = mcp
        .call_tool(
            &rw_ctx,
            "wiki_get",
            json!({ "file": "wiki/entities/voice-agent.md" }),
        )
        .await
        .expect("wiki_get should return written page");
    assert!(
        get_res
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .contains("Streamable HTTP")
    );

    let stage_res = mcp
        .call_tool(
            &rw_ctx,
            "add_to_sources",
            json!({
                "topic_folder": "architecture/mcp",
                "title": "Knowledge MCP Design Notes",
                "markdown_body": "Knowledge MCP exposes WikiEngine tools over /mcp/knowledge with Bearer token authentication and schema sanitization.",
                "provenance": "user"
            }),
        )
        .await
        .expect("add_to_sources should succeed for ReadWrite");
    assert_eq!(
        stage_res
            .pointer("/metadata/autoflow_triggered")
            .and_then(Value::as_bool),
        Some(true)
    );
}

#[test]
fn runtime_token_lifecycle_redaction_and_dynamic_concurrency() {
    let (dir, engine) = test_engine();
    let cfg = WikiMcpConfig::default();
    let mcp = KnowledgeMcp::from_config(engine, None, &cfg, Some(&dir.0));
    assert!(!mcp.is_enabled());

    let snap = mcp
        .update_settings(
            UpdateKnowledgeMcpInput {
                enabled: true,
                allow_write: true,
                max_concurrency: Some(1),
                call_timeout_ms: Some(15_000),
                max_result_chars: Some(8_000),
                expected_revision: Some(1),
            },
            None,
        )
        .expect("update settings");
    assert_eq!(snap["enabled"], Value::Bool(true));
    assert_eq!(snap["revision"], json!(2));

    // Stale revision must be rejected with RevisionConflict
    let conflict = mcp
        .update_settings(
            UpdateKnowledgeMcpInput {
                enabled: true,
                allow_write: false,
                max_concurrency: Some(2),
                call_timeout_ms: None,
                max_result_chars: None,
                expected_revision: Some(1),
            },
            None,
        )
        .expect_err("stale revision must fail");
    assert_eq!(conflict, KnowledgeMcpAdminError::RevisionConflict);

    // Dynamic concurrency limit = 1 permits 1 active request and rejects the 2nd until dropped
    let p1 = mcp
        .try_acquire_permit()
        .expect("first permit should succeed");
    assert!(mcp.try_acquire_permit().is_none());
    drop(p1);
    let _p2 = mcp
        .try_acquire_permit()
        .expect("permit should succeed after drop");

    let (entry, snapshot_after_create) = mcp
        .create_token("Web Client".into(), KnowledgeMcpAccess::ReadOnly, None)
        .expect("create token");
    assert!(entry.token.starts_with("kmcp_"));

    // Debug output must redact raw secret token
    let debug_str = format!("{entry:?}");
    assert!(debug_str.contains("[REDACTED]"));
    assert!(!debug_str.contains(&entry.token));

    // Admin snapshot must return masked_token and never leak raw token
    let snapshot_json = serde_json::to_string(&snapshot_after_create).unwrap();
    assert!(!snapshot_json.contains(&entry.token));
    assert!(
        snapshot_after_create["tokens"][0]["masked_token"]
            .as_str()
            .unwrap_or_default()
            .contains("••••••••")
    );

    let verified = mcp
        .verify_token(&entry.token)
        .expect("verify newly created token");
    assert_eq!(verified.access, KnowledgeMcpAccess::ReadOnly);

    mcp.delete_token(&entry.id).expect("delete token");
    assert!(mcp.verify_token(&entry.token).is_none());
}

#[tokio::test]
async fn wiki_write_tools_normalize_markdown_tables_for_external_markdown_apps() {
    let (_dir, engine) = test_engine();
    let mcp = KnowledgeMcp::new(Arc::clone(&engine), None, true, 12_000);
    let rw_ctx = test_ctx(KnowledgeMcpAccess::ReadWrite);

    let raw_content = [
        "---",
        "title: \"FVG Synthesis\"",
        "category: \"synthesis\"",
        "---",
        "",
        "# FVG Synthesis",
        "",
        "Đoạn văn có [[wiki/concepts/trading/fair-value-gap|Fair Value Gap (FVG)]] ngoài bảng.",
        "",
        "## Tiến triển bằng chứng theo nguồn",
        "| Nguồn | Luận điểm chính | Ghi chú |",
        "|---|---|---|",
        "| [[wiki/sources/trading/fvg/the-secret-fvg-breakaway-gap|The Secret FVG that we don't trade back into]] | Breakaway Gap | `a | b` |",
        "| [[wiki/sources/trading/sharp-turn-entry/the-sharp-turn-entry-model\\|The Sharp Turn Entry Model]] | Đã escape sẵn | Giữ nguyên |",
        "Đoạn văn ngay sau bảng không có dòng trống.",
        "",
        "```markdown",
        "| [[not/escaped/in/code|Code Block Link]] | Value |",
        "```",
    ]
    .join("\n");

    mcp.call_tool(
        &rw_ctx,
        "wiki_write_file",
        json!({
            "scope": "wiki",
            "rel_path": "syntheses/trading/fvg-synthesis.md",
            "content": raw_content,
        }),
    )
    .await
    .expect("wiki_write_file should succeed");

    let saved = engine
        .get_document(
            "wiki/syntheses/trading/fvg-synthesis.md",
            llm_wiki::GetOptions {
                include_frontmatter: true,
                ..Default::default()
            },
        )
        .expect("read saved wiki synthesis")
        .content;

    // 1. Prose WikiLink outside table keeps unescaped pipe `|`
    assert!(saved.contains("Đoạn văn có [[wiki/concepts/trading/fair-value-gap|Fair Value Gap (FVG)]] ngoài bảng."));
    // 2. Blank line inserted between heading and table header, and after table before paragraph
    assert!(saved.contains("## Tiến triển bằng chứng theo nguồn\n\n| Nguồn | Luận điểm chính | Ghi chú |"));
    assert!(saved.contains("| Đã escape sẵn | Giữ nguyên |\n\nĐoạn văn ngay sau bảng không có dòng trống."));
    // 3. Unescaped WikiLink pipe inside table cell is escaped as `\|` for Obsidian / external GFM apps
    assert!(saved.contains(
        "| [[wiki/sources/trading/fvg/the-secret-fvg-breakaway-gap\\|The Secret FVG that we don't trade back into]] | Breakaway Gap | `a | b` |"
    ));
    // 4. Already-escaped WikiLink inside table cell is not double-escaped
    assert!(saved.contains(
        "| [[wiki/sources/trading/sharp-turn-entry/the-sharp-turn-entry-model\\|The Sharp Turn Entry Model]] | Đã escape sẵn | Giữ nguyên |"
    ));
    assert!(!saved.contains("\\\\|"));
    // 5. Fenced code block content is untouched
    assert!(saved.contains("| [[not/escaped/in/code|Code Block Link]] | Value |"));
}
