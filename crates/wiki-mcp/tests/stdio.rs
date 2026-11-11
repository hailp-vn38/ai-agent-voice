use axum::{
    Router,
    extract::Request,
    http::{StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use rmcp::{
    ErrorData, RoleServer, ServerHandler, ServiceExt,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ListToolsResult,
        PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
    },
    service::RequestContext,
    transport::{
        child_process::TokioChildProcess,
        streamable_http_server::{
            StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
        },
    },
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tempfile::TempDir;
use tokio::process::Command;
use wiki_core::{Vault, VaultConfig};
use wiki_mcp::{Access, CallCtx, Policy, ToolHost};

#[derive(Clone)]
struct FakeAppMcp;

impl ServerHandler for FakeAppMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
    }

    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        if request
            .as_ref()
            .is_some_and(|params| params.cursor.is_some())
        {
            return Err(ErrorData::invalid_params("invalid_cursor", None));
        }
        Ok(ListToolsResult {
            tools: vec![
                Tool::new(
                    "wiki_status",
                    "status from app",
                    Arc::new(serde_json::Map::new()),
                ),
                Tool::new(
                    "wiki_write_file",
                    "write through app",
                    Arc::new(serde_json::Map::new()),
                ),
            ],
            ..Default::default()
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let response = if request.name == "wiki_status" {
            serde_json::json!({"tool": "wiki_status", "source": "desktop-app"})
        } else {
            serde_json::json!({"tool": request.name, "arguments": request.arguments})
        };
        Ok(CallToolResult::structured(response).into())
    }
}

async fn require_app_token(expected: &'static str, request: Request, next: Next) -> Response {
    if request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        != Some(format!("Bearer {expected}").as_str())
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    next.run(request).await
}

/// Fake desktop app that only accepts `expected_token`.
async fn start_fake_app(expected_token: &'static str) -> (u16, tokio::task::JoinHandle<()>) {
    let (port, server, _) = start_counting_fake_app(expected_token).await;
    (port, server)
}

/// Like `start_fake_app`, also counting the MCP sessions clients open.
async fn start_counting_fake_app(
    expected_token: &'static str,
) -> (
    u16,
    tokio::task::JoinHandle<()>,
    Arc<std::sync::atomic::AtomicUsize>,
) {
    let sessions = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let opened = sessions.clone();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let config = StreamableHttpServerConfig::default()
        // Session mode like the desktop app, so one handler per client session.
        .with_legacy_session_mode(true)
        .with_json_response(true)
        .with_allowed_hosts([format!("127.0.0.1:{port}")]);
    let service = StreamableHttpService::new(
        move || {
            opened.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(FakeAppMcp)
        },
        Arc::new(LocalSessionManager::default()),
        config,
    );
    let app = Router::new()
        .route_service("/mcp", service)
        .layer(middleware::from_fn(move |request, next| {
            require_app_token(expected_token, request, next)
        }));
    (
        port,
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() }),
        sessions,
    )
}

fn write_runtime(path: &std::path::Path, contents: serde_json::Value) {
    std::fs::write(path, contents.to_string()).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
}

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
    let writer = Arc::new(Vault::open(VaultConfig::new(root.path())).unwrap());
    assert!(writer.is_writable());
    let app_host = ToolHost::new(writer, Policy::default());
    app_host
        .call(
            CallCtx {
                access: Access::ReadWrite,
                cancel: tokio_util::sync::CancellationToken::new(),
                deadline: tokio::time::Instant::now() + Duration::from_secs(30),
            },
            "wiki_write_file",
            serde_json::json!({
                "scope": "wiki",
                "rel_path": "app-write.md",
                "content": "# App writer\n"
            }),
        )
        .await
        .unwrap();

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
    assert!(root.path().join("wiki/app-write.md").exists());
    assert!(root.path().join("wiki/index.md").exists());
    client.cancel().await.unwrap();
}

#[tokio::test]
async fn stdio_bridges_tool_list_and_status_to_the_running_app() {
    let runtime_dir = TempDir::new().unwrap();
    let runtime_file = runtime_dir.path().join("runtime.json");
    let (port, server) = start_fake_app("app-read-token").await;
    write_runtime(
        &runtime_file,
        serde_json::json!({
            "port": port,
            "readToken": "app-read-token",
            "writeToken": "app-write-token"
        }),
    );

    let mut binary = Command::new(env!("CARGO_BIN_EXE_wiki-mcp"));
    binary.arg("--runtime-file").arg(&runtime_file);
    let transport = TokioChildProcess::new(binary).unwrap();
    let client = ().serve(transport).await.unwrap();
    let tools = client.list_all_tools().await.unwrap();
    assert_eq!(
        tools
            .iter()
            .filter(|tool| tool.name == "wiki_status")
            .count(),
        1
    );
    let result = client
        .call_tool(CallToolRequestParams::new("wiki_status"))
        .await
        .unwrap();
    assert_eq!(result.structured_content.unwrap()["source"], "desktop-app");
    let write = client
        .call_tool(
            CallToolRequestParams::new("wiki_write_file").with_arguments(
                serde_json::json!({"scope":"wiki","rel_path":"bridge.md","content":"# Bridged"})
                    .as_object()
                    .unwrap()
                    .clone(),
            ),
        )
        .await
        .unwrap();
    let write = write.structured_content.unwrap();
    assert_eq!(write["tool"], "wiki_write_file");
    assert_eq!(write["arguments"]["rel_path"], "bridge.md");
    client.cancel().await.unwrap();
    server.abort();
}

#[tokio::test]
async fn dead_runtime_endpoint_falls_back_to_direct_stdio_within_timeout() {
    let vault = TempDir::new().unwrap();
    let runtime_dir = TempDir::new().unwrap();
    let runtime_file = runtime_dir.path().join("runtime.json");
    write_runtime(
        &runtime_file,
        serde_json::json!({
            "port": 9,
            "readToken": "stale-read-token",
            "writeToken": "stale-write-token"
        }),
    );

    let start = Instant::now();
    let transport = TokioChildProcess::new({
        let mut command = command(&vault);
        command.arg("--runtime-file").arg(&runtime_file);
        command
    })
    .unwrap();
    let client = tokio::time::timeout(Duration::from_secs(5), ().serve(transport))
        .await
        .expect("fallback should finish within five seconds")
        .unwrap();
    assert!(start.elapsed() < Duration::from_secs(5));
    let status = client
        .call_tool(CallToolRequestParams::new("wiki_status"))
        .await
        .unwrap();
    assert_eq!(status.structured_content.unwrap()["tool"], "wiki_status");
    client.cancel().await.unwrap();
}

async fn bridged_status_source(mut binary: Command) -> serde_json::Value {
    binary.stderr(std::process::Stdio::null());
    let transport = TokioChildProcess::new(binary).unwrap();
    let client = tokio::time::timeout(Duration::from_secs(5), ().serve(transport))
        .await
        .expect("wiki-mcp should start within five seconds")
        .unwrap();
    let status = client
        .call_tool(CallToolRequestParams::new("wiki_status"))
        .await
        .unwrap();
    client.cancel().await.unwrap();
    status.structured_content.unwrap()["source"].clone()
}

fn runtime_for(path: &std::path::Path, port: u16, vault: &std::path::Path) {
    write_runtime(
        path,
        serde_json::json!({
            "port": port,
            "vaultPath": std::fs::canonicalize(vault).unwrap(),
            "readToken": "app-read-token",
            "writeToken": "app-write-token"
        }),
    );
}

#[tokio::test]
async fn bridge_sends_write_token_only_with_allow_write() {
    let vault = TempDir::new().unwrap();
    let runtime_dir = TempDir::new().unwrap();
    let runtime_file = runtime_dir.path().join("runtime.json");
    let (port, server) = start_fake_app("app-write-token").await;
    runtime_for(&runtime_file, port, vault.path());

    let mut writer = command(&vault);
    writer.arg("--runtime-file").arg(&runtime_file);
    assert_eq!(bridged_status_source(writer).await, "desktop-app");

    // Without --allow-write the bridge presents the read token, which this app
    // rejects, so wiki-mcp falls back to the vault directly.
    let mut reader = Command::new(env!("CARGO_BIN_EXE_wiki-mcp"));
    reader
        .arg("--vault")
        .arg(vault.path())
        .arg("--runtime-file")
        .arg(&runtime_file);
    assert_ne!(bridged_status_source(reader).await, "desktop-app");
    server.abort();
}

#[tokio::test]
async fn bridge_only_serves_the_vault_named_by_vault_flag() {
    let requested = TempDir::new().unwrap();
    let other = TempDir::new().unwrap();
    let runtime_dir = TempDir::new().unwrap();
    let runtime_file = runtime_dir.path().join("runtime.json");
    let (port, server) = start_fake_app("app-read-token").await;

    let pinned = || {
        let mut binary = Command::new(env!("CARGO_BIN_EXE_wiki-mcp"));
        binary
            .arg("--vault")
            .arg(requested.path())
            .arg("--runtime-file")
            .arg(&runtime_file);
        binary
    };

    runtime_for(&runtime_file, port, other.path());
    assert_ne!(bridged_status_source(pinned()).await, "desktop-app");

    runtime_for(&runtime_file, port, requested.path());
    assert_eq!(bridged_status_source(pinned()).await, "desktop-app");
    server.abort();
}

#[tokio::test]
async fn bridge_keeps_one_app_session_for_the_whole_stdio_session() {
    let runtime_dir = TempDir::new().unwrap();
    let runtime_file = runtime_dir.path().join("runtime.json");
    let (port, server, sessions) = start_counting_fake_app("app-read-token").await;
    write_runtime(
        &runtime_file,
        serde_json::json!({
            "port": port,
            "readToken": "app-read-token",
            "writeToken": "app-write-token"
        }),
    );
    let mut binary = Command::new(env!("CARGO_BIN_EXE_wiki-mcp"));
    binary.arg("--runtime-file").arg(&runtime_file);
    let client = ().serve(TokioChildProcess::new(binary).unwrap()).await.unwrap();
    client.list_all_tools().await.unwrap();
    for _ in 0..3 {
        client
            .call_tool(CallToolRequestParams::new("wiki_status"))
            .await
            .unwrap();
    }
    assert_eq!(sessions.load(std::sync::atomic::Ordering::SeqCst), 1);
    client.cancel().await.unwrap();
    server.abort();
}

#[tokio::test]
async fn concurrent_writers_keep_the_index_whole_and_only_the_lockless_one_is_read_only() {
    let root = TempDir::new().unwrap();
    let app = Arc::new(Vault::open(VaultConfig::new(root.path())).unwrap());
    assert!(app.is_writable());
    let client = ().serve(TokioChildProcess::new(command(&root)).unwrap()).await.unwrap();

    const WRITES: usize = 12;
    let app_writes = {
        let app = app.clone();
        tokio::task::spawn_blocking(move || {
            (0..WRITES)
                .map(|n| {
                    app.apply(wiki_core::Change::Write {
                        path: wiki_core::DocPath::parse(
                            wiki_core::Scope::Wiki,
                            format!("app/page-{n}.md"),
                        )
                        .unwrap(),
                        content: format!("# App page {n}\n\nconcurrentmarker{n}"),
                        expect: None,
                    })
                })
                .collect::<Vec<_>>()
        })
    };
    let mcp_writes = async {
        let mut results = Vec::new();
        for n in 0..WRITES {
            results.push(
                client
                    .call_tool(
                        CallToolRequestParams::new("wiki_write_file").with_arguments(
                            serde_json::json!({
                                "scope": "wiki",
                                "rel_path": format!("mcp/page-{n}.md"),
                                "content": format!("# MCP page {n}"),
                            })
                            .as_object()
                            .unwrap()
                            .clone(),
                        ),
                    )
                    .await
                    .unwrap(),
            );
        }
        results
    };
    let (app_writes, mcp_writes) = tokio::join!(app_writes, mcp_writes);

    assert!(app_writes.unwrap().iter().all(Result::is_ok));
    assert!(mcp_writes.iter().all(|result| {
        result.is_error.unwrap_or(false)
            && result.content.iter().any(|block| {
                block
                    .as_text()
                    .is_some_and(|text| text.text.contains("read-only"))
            })
    }));
    assert!(!root.path().join("wiki/mcp").exists());
    client.cancel().await.unwrap();
    drop(app);

    // Reopen from disk: the index holds every app page and nothing else broke.
    let reopened = Vault::open(VaultConfig::new(root.path())).unwrap();
    for n in 0..WRITES {
        let marker = format!("concurrentmarker{n}");
        assert!(
            reopened
                .search(&marker)
                .unwrap()
                .iter()
                .any(|hit| hit.path == format!("wiki/app/page-{n}.md")),
            "index lost app page {n}"
        );
    }
    let index_md = std::fs::read_to_string(root.path().join("wiki/index.md")).unwrap();
    assert!((0..WRITES).all(|n| index_md.contains(&format!("page-{n}"))));
}
