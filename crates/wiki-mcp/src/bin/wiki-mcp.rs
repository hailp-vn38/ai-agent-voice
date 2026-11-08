use rmcp::{
    ErrorData, RoleClient, RoleServer, ServerHandler, ServiceExt,
    model::{
        CallToolRequest, CallToolRequestParams, CallToolResponse, CallToolResult,
        CancelledNotificationParam, ClientConfig, ClientRequest, ContentBlock, ListToolsResult,
        PaginatedRequestParams, ServerCapabilities, ServerConfig, ServerResult, Tool,
    },
    service::{Peer, PeerRequestOptions, RequestContext, RunningService},
    transport::{
        StreamableHttpClientTransport, stdio,
        streamable_http_client::StreamableHttpClientTransportConfig,
    },
};
use serde::Deserialize;
use std::{
    path::{Path, PathBuf},
    process::ExitCode,
    sync::Arc,
    time::Duration,
};
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
    #[serde(default)]
    vault_path: Option<PathBuf>,
    read_token: String,
    write_token: String,
}

#[derive(Clone)]
struct BridgeConfig {
    uri: String,
    token: String,
}

/// One MCP session to the running app for the whole stdio session, so the
/// app sees one client and cancellations reach the request they belong to.
struct Bridge {
    config: BridgeConfig,
    client: tokio::sync::Mutex<Option<RunningService<RoleClient, ClientConfig>>>,
}

impl Bridge {
    /// The live session, reconnecting once if the app restarted its server.
    async fn peer(&self) -> Result<Peer<RoleClient>, ErrorData> {
        let mut client = self.client.lock().await;
        if let Some(running) = client.as_ref()
            && !running.is_closed()
            && !running.peer().is_transport_closed()
        {
            return Ok(running.peer().clone());
        }
        let running = bridge_client(&self.config)
            .await
            .map_err(|error| ErrorData::internal_error(error, None))?;
        let peer = running.peer().clone();
        *client = Some(running);
        Ok(peer)
    }

    /// Ends the HTTP session so the app stops counting this client.
    async fn close(&self) {
        if let Some(running) = self.client.lock().await.take() {
            let _ = running.cancel().await;
        }
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<CallToolResponse, ErrorData> {
        let peer = self.peer().await?;
        let handle = peer
            .send_cancellable_request(
                ClientRequest::CallToolRequest(CallToolRequest::new(request)),
                PeerRequestOptions::no_options(),
            )
            .await
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
        let request_id = handle.id.clone();
        let response = tokio::select! {
            response = handle.await_response() => response,
            () = cancel.cancelled() => {
                let _ = peer
                    .notify_cancelled(CancelledNotificationParam::new(
                        Some(request_id),
                        Some("cancelled by the stdio client".into()),
                    ))
                    .await;
                return Err(ErrorData::internal_error("request cancelled", None));
            }
        };
        match response.map_err(|error| ErrorData::internal_error(error.to_string(), None))? {
            ServerResult::CallToolResult(result) => Ok(CallToolResponse::Complete(result)),
            ServerResult::InputRequiredResult(result) => {
                Ok(CallToolResponse::InputRequired(result))
            }
            ServerResult::CreateTaskResult(result) => Ok(CallToolResponse::Task(result)),
            _ => Err(ErrorData::internal_error(
                "unexpected response from the Wiki app",
                None,
            )),
        }
    }
}

#[derive(Clone)]
enum Backend {
    Direct(Arc<ToolHost>),
    Bridge(Arc<Bridge>),
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
    home.join("Library/Application Support")
        .join(bundle_identifier().unwrap_or_else(|| DEFAULT_BUNDLE_ID.into()))
        .join("runtime.json")
}

/// Bundle id used when `wiki-mcp` runs outside `Wiki.app` (development).
const DEFAULT_BUNDLE_ID: &str = "com.vela.wikidesktop";

/// The app keeps `runtime.json` under its bundle id, which a release may
/// override, so read it from the `Wiki.app` this binary is bundled in.
fn bundle_identifier() -> Option<String> {
    let executable = std::env::current_exe().ok()?;
    let contents = executable.parent()?.parent()?;
    let plist = std::fs::read_to_string(contents.join("Info.plist")).ok()?;
    bundle_identifier_from_plist(&plist)
}

fn bundle_identifier_from_plist(plist: &str) -> Option<String> {
    let (_, after_key) = plist.split_once("<key>CFBundleIdentifier</key>")?;
    let (_, after_open) = after_key.split_once("<string>")?;
    let (value, _) = after_open.split_once("</string>")?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

/// The running app serves exactly one vault. A client pinned to a vault with
/// `--vault` may only be bridged when the app has that same vault open.
fn runtime_serves_requested_vault(runtime: &RuntimeFile, requested: Option<&Path>) -> bool {
    let Some(requested) = requested else {
        return true;
    };
    let Some(served) = runtime.vault_path.as_deref() else {
        return false;
    };
    match (
        std::fs::canonicalize(requested),
        std::fs::canonicalize(served),
    ) {
        (Ok(requested), Ok(served)) => requested == served,
        _ => false,
    }
}

/// Bridged requests carry the same access the client would get in direct mode:
/// the write token only with `--allow-write`, otherwise the read token.
fn bridge_token(runtime: RuntimeFile, allow_write: bool) -> String {
    if allow_write && !runtime.write_token.is_empty() {
        runtime.write_token
    } else {
        runtime.read_token
    }
}

async fn bridge_backend(path: PathBuf, args: &Args) -> Option<Bridge> {
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
    if !runtime_serves_requested_vault(&runtime, args.vault.as_deref()) {
        eprintln!(
            "wiki-mcp: the running Wiki app has a different vault open; using --vault directly"
        );
        return None;
    }
    let bridge = BridgeConfig {
        uri: format!("http://127.0.0.1:{}/mcp", runtime.port),
        token: bridge_token(runtime, args.allow_write),
    };
    // The probe session becomes the bridge's session.
    let client = bridge_client(&bridge).await.ok()?;
    Some(Bridge {
        config: bridge,
        client: tokio::sync::Mutex::new(Some(client)),
    })
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
            Backend::Bridge(bridge) => bridge
                .peer()
                .await?
                .list_tools(request)
                .await
                .map_err(|error| ErrorData::internal_error(error.to_string(), None)),
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
            Backend::Bridge(bridge) => bridge.call_tool(request, ctx.ct).await,
        }
    }
}

async fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    let backend = match bridge_backend(runtime_file_path(&args), &args).await {
        Some(bridge) => {
            eprintln!("wiki-mcp: forwarding stdio requests to the running Wiki app");
            Backend::Bridge(Arc::new(bridge))
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
    let bridge = match &backend {
        Backend::Bridge(bridge) => Some(bridge.clone()),
        Backend::Direct(_) => None,
    };
    let service = StdioHandler { backend };
    let running = service.serve(stdio()).await?;
    let waited = running.waiting().await;
    if let Some(bridge) = bridge {
        bridge.close().await;
    }
    waited?;
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

#[cfg(test)]
mod tests {
    use super::bundle_identifier_from_plist;

    #[test]
    fn reads_the_bundle_identifier_from_info_plist() {
        let plist = r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
  <key>CFBundleExecutable</key><string>wiki-desktop</string>
  <key>CFBundleIdentifier</key>
  <string>com.example.wiki-beta</string>
</dict></plist>"#;
        assert_eq!(
            bundle_identifier_from_plist(plist).as_deref(),
            Some("com.example.wiki-beta")
        );
        assert_eq!(
            bundle_identifier_from_plist("<plist><dict></dict></plist>"),
            None
        );
    }
}
