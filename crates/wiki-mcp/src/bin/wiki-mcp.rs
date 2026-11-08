use rmcp::{
    ErrorData, RoleClient, RoleServer, ServerHandler, ServiceExt,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ClientConfig, ContentBlock,
        ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
    },
    service::RequestContext,
    transport::{
        StreamableHttpClientTransport, stdio,
        streamable_http_client::StreamableHttpClientTransportConfig,
    },
};
use serde::Deserialize;
use std::{path::PathBuf, process::ExitCode, sync::Arc, time::Duration};
use wiki_core::{Vault, VaultConfig};
use wiki_mcp::{Access, CallCtx, Policy, ToolHost};

const BRIDGE_CONNECT_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Debug)]
struct Args {
    vault: Option<PathBuf>,
    allow_write: bool,
    runtime_file: Option<PathBuf>,
}

fn parse_args() -> Result<Option<Args>, String> {
    let mut args = std::env::args_os().skip(1);
    let mut vault = None;
    let mut allow_write = false;
    let mut runtime_file = None;
    while let Some(argument) = args.next() {
        match argument.to_str() {
            Some("--vault") => {
                let path = args.next().ok_or("--vault requires a directory")?;
                vault = Some(PathBuf::from(path));
            }
            Some("--allow-write") => allow_write = true,
            Some("--runtime-file") => {
                let path = args.next().ok_or("--runtime-file requires a path")?;
                runtime_file = Some(PathBuf::from(path));
            }
            Some("--help" | "-h") => return Ok(None),
            _ => return Err(format!("unknown argument: {}", argument.to_string_lossy())),
        }
    }
    let vault = vault.or_else(|| std::env::var_os("WIKI_VAULT_PATH").map(PathBuf::from));
    Ok(Some(Args {
        vault,
        allow_write,
        runtime_file,
    }))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeFile {
    port: u16,
    read_token: String,
    write_token: String,
}

#[derive(Clone)]
struct BridgeConfig {
    uri: String,
    token: String,
}

#[derive(Clone)]
enum Backend {
    Direct(Arc<ToolHost>),
    Bridge(BridgeConfig),
}

#[derive(Clone)]
struct StdioHandler {
    backend: Backend,
}

async fn bridge_client(
    bridge: &BridgeConfig,
) -> Result<rmcp::service::RunningService<RoleClient, ClientConfig>, String> {
    let transport = StreamableHttpClientTransport::with_client(
        reqwest::Client::new(),
        StreamableHttpClientTransportConfig::with_uri(bridge.uri.clone())
            .auth_header(bridge.token.clone()),
    );
    tokio::time::timeout(
        BRIDGE_CONNECT_TIMEOUT,
        ClientConfig::default().serve(transport),
    )
    .await
    .map_err(|_| "bridge connection timed out".to_string())?
    .map_err(|error| error.to_string())
}

fn runtime_file_path(args: &Args) -> PathBuf {
    if let Some(path) = &args.runtime_file {
        return path.clone();
    }
    if let Some(path) = std::env::var_os("WIKI_DESKTOP_RUNTIME_FILE") {
        return PathBuf::from(path);
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join("Library/Application Support/com.vela.wikidesktop/runtime.json")
}

async fn bridge_backend(path: PathBuf) -> Option<BridgeConfig> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if std::fs::metadata(&path).ok()?.permissions().mode() & 0o077 != 0 {
            return None;
        }
    }
    let runtime = tokio::fs::read(path).await.ok()?;
    let runtime: RuntimeFile = serde_json::from_slice(&runtime).ok()?;
    if runtime.port == 0 || runtime.read_token.is_empty() {
        return None;
    }
    let bridge = BridgeConfig {
        uri: format!("http://127.0.0.1:{}/mcp", runtime.port),
        token: if runtime.write_token.is_empty() {
            runtime.read_token
        } else {
            runtime.write_token
        },
    };
    let client = bridge_client(&bridge).await.ok()?;
    let _ = client.cancel().await;
    Some(bridge)
}

async fn connect_bridge(
    bridge: &BridgeConfig,
) -> Result<rmcp::service::RunningService<RoleClient, ClientConfig>, ErrorData> {
    bridge_client(bridge)
        .await
        .map_err(|error| ErrorData::internal_error(error, None))
}

impl ServerHandler for StdioHandler {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build()).with_server_info(
            rmcp::model::Implementation::new("wiki-mcp", env!("CARGO_PKG_VERSION")),
        )
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
        match &self.backend {
            Backend::Direct(host) => Ok(ListToolsResult {
                tools: host
                    .list_tools(Access::ReadWrite)
                    .into_iter()
                    .map(|tool| {
                        Tool::new(
                            tool.name,
                            tool.description,
                            tool.parameters.as_object().cloned().unwrap_or_default(),
                        )
                    })
                    .collect(),
                ..Default::default()
            }),
            Backend::Bridge(bridge) => {
                let client = connect_bridge(bridge).await?;
                let result = client
                    .list_tools(request)
                    .await
                    .map_err(|error| ErrorData::internal_error(error.to_string(), None));
                let _ = client.cancel().await;
                result
            }
        }
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        match &self.backend {
            Backend::Direct(host) => {
                let result = host
                    .call(
                        CallCtx {
                            access: Access::ReadWrite,
                            cancel: ctx.ct,
                            deadline: tokio::time::Instant::now() + Duration::from_secs(30),
                        },
                        &request.name,
                        serde_json::Value::Object(request.arguments.unwrap_or_default()),
                    )
                    .await;
                let result = match result {
                    Ok(value) => CallToolResult::structured(value),
                    Err(error) => {
                        CallToolResult::error(vec![ContentBlock::text(error.to_string())])
                    }
                };
                Ok(result.into())
            }
            Backend::Bridge(bridge) => {
                let client = connect_bridge(bridge).await?;
                let result = client
                    .call_tool(request)
                    .await
                    .map_err(|error| ErrorData::internal_error(error.to_string(), None));
                let _ = client.cancel().await;
                result.map(Into::into)
            }
        }
    }
}

async fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    let backend = match bridge_backend(runtime_file_path(&args)).await {
        Some(bridge) => {
            eprintln!("wiki-mcp: forwarding stdio requests to the running Wiki app");
            Backend::Bridge(bridge)
        }
        None => {
            let vault_path = args
                .vault
                .ok_or("pass --vault <directory> or set WIKI_VAULT_PATH when the app is closed")?;
            let vault = Arc::new(Vault::open(VaultConfig::new(vault_path))?);
            if !vault.is_writable() {
                eprintln!(
                    "wiki-mcp: vault is open read-only because another writer holds its lock"
                );
            }
            Backend::Direct(Arc::new(ToolHost::new(
                vault,
                Policy {
                    allow_write: args.allow_write,
                    ..Policy::default()
                },
            )))
        }
    };
    let service = StdioHandler { backend };
    let running = service.serve(stdio()).await?;
    running.waiting().await?;
    Ok(())
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(Some(args)) => args,
        Ok(None) => {
            eprintln!(
                "Usage: wiki-mcp --vault <directory> [--allow-write] [--runtime-file <path>]\n       WIKI_VAULT_PATH may replace --vault"
            );
            return ExitCode::SUCCESS;
        }
        Err(error) => {
            eprintln!("wiki-mcp: {error}");
            return ExitCode::from(2);
        }
    };
    match run(args).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("wiki-mcp: {error}");
            ExitCode::FAILURE
        }
    }
}
