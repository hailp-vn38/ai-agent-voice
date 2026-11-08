use axum::{
    Router,
    extract::{Request, State},
    http::{StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use keyring::Entry;
use rmcp::{
    ErrorData, RoleServer, ServerHandler,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ListToolsResult,
        PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
    },
    service::RequestContext,
    transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    },
};
use serde::{Deserialize, Serialize};
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use wiki_core::Vault;
use wiki_mcp::{Access, CallCtx, Policy, ToolHost};

const KEYCHAIN_SERVICE: &str = "com.vela.wikidesktop.mcp";
const MCP_PATH: &str = "/mcp";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpStatus {
    pub active_requests: usize,
    pub last_call_at: Option<u64>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpInfo {
    pub port: u16,
    pub allow_write: bool,
    pub read_url: String,
    pub write_url: String,
    pub read_config: String,
    pub write_config: String,
    pub stdio_config: String,
    pub status: McpStatus,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeFile {
    port: u16,
    read_token: String,
    write_token: String,
}

#[derive(Clone, Copy)]
struct AuthenticatedAccess(Access);

struct ServerState {
    emit_status: Arc<dyn Fn(McpStatus) + Send + Sync>,
    host: Arc<ToolHost>,
    read_token: std::sync::RwLock<String>,
    write_token: std::sync::RwLock<String>,
    allow_write: AtomicBool,
    port: u16,
    active_requests: AtomicUsize,
    last_call_at: std::sync::RwLock<Option<u64>>,
}

impl ServerState {
    fn status(&self) -> McpStatus {
        McpStatus {
            active_requests: self.active_requests.load(Ordering::Acquire),
            last_call_at: *self.last_call_at.read().expect("MCP status lock"),
        }
    }

    fn publish_status(&self) {
        (self.emit_status)(self.status());
    }

    fn set_allow_write(&self, allow: bool) {
        self.allow_write.store(allow, Ordering::Release);
        self.host.update_policy(Policy {
            allow_write: allow,
            ..Policy::default()
        });
    }

    fn update_token(&self, write: bool, token: String) -> Result<(), String> {
        let name = if write { "write" } else { "read" };
        keychain_entry(name).and_then(|entry| {
            entry
                .set_password(&token)
                .map_err(|error| error.to_string())
        })?;
        let destination = if write {
            &self.write_token
        } else {
            &self.read_token
        };
        *destination
            .write()
            .map_err(|_| "MCP token lock poisoned".to_string())? = token;
        Ok(())
    }

    fn refresh_runtime_file(&self, runtime_file: &Path) -> Result<(), String> {
        write_runtime_file(
            runtime_file,
            &RuntimeFile {
                port: self.port,
                read_token: self
                    .read_token
                    .read()
                    .map_err(|_| "read token lock poisoned")?
                    .clone(),
                write_token: self
                    .write_token
                    .read()
                    .map_err(|_| "write token lock poisoned")?
                    .clone(),
            },
        )
    }
}

pub struct McpServerRuntime {
    state: Arc<ServerState>,
    cancellation: CancellationToken,
    runtime_file: PathBuf,
    vault_path: PathBuf,
}

impl McpServerRuntime {
    pub fn info(&self) -> Result<McpInfo, String> {
        let base = format!("http://127.0.0.1:{}{MCP_PATH}", self.state.port);
        let read_token = self
            .state
            .read_token
            .read()
            .map_err(|_| "read token lock poisoned")?;
        let write_token = self
            .state
            .write_token
            .read()
            .map_err(|_| "write token lock poisoned")?;
        let executable = std::env::current_exe().map_err(|error| error.to_string())?;
        let sidecar = executable
            .parent()
            .unwrap_or_else(|| Path::new("/"))
            .join("wiki-mcp");
        let stdio_config = serde_json::json!({
            "mcpServers": {
                "wiki": {
                    "command": sidecar,
                    "args": ["--vault", self.vault_path]
                }
            }
        })
        .to_string();
        Ok(McpInfo {
            port: self.state.port,
            allow_write: self.state.allow_write.load(Ordering::Acquire),
            read_url: base.clone(),
            write_url: base.clone(),
            read_config: serde_json::json!({"url": base, "headers": {"Authorization": format!("Bearer {read_token}")}}).to_string(),
            write_config: serde_json::json!({"url": format!("http://127.0.0.1:{}{MCP_PATH}", self.state.port), "headers": {"Authorization": format!("Bearer {write_token}")}}).to_string(),
            stdio_config,
            status: self.state.status(),
        })
    }

    pub fn set_allow_write(&self, allow: bool) -> Result<(), String> {
        let previous = self.state.allow_write.load(Ordering::Acquire);
        self.state.set_allow_write(allow);
        if let Err(error) = self.state.refresh_runtime_file(&self.runtime_file) {
            self.state.set_allow_write(previous);
            return Err(error);
        }
        self.state.publish_status();
        Ok(())
    }

    pub fn rotate_token(&self, write: bool) -> Result<(), String> {
        let previous = if write {
            self.state
                .write_token
                .read()
                .map_err(|_| "write token lock poisoned")?
                .clone()
        } else {
            self.state
                .read_token
                .read()
                .map_err(|_| "read token lock poisoned")?
                .clone()
        };
        self.state
            .update_token(write, uuid::Uuid::new_v4().to_string())?;
        if let Err(error) = self.state.refresh_runtime_file(&self.runtime_file) {
            let _ = self.state.update_token(write, previous);
            return Err(error);
        }
        Ok(())
    }

    pub fn shutdown(&self) {
        self.cancellation.cancel();
        let _ = std::fs::remove_file(&self.runtime_file);
    }
}

pub async fn start(
    app: AppHandle,
    vault: Arc<Vault>,
    requested_port: Option<u16>,
    allow_write: bool,
    runtime_file: PathBuf,
    vault_path: PathBuf,
) -> Result<McpServerRuntime, String> {
    let emit_app = app.clone();
    let emit_status = Arc::new(move |status| {
        let _ = emit_app.emit("mcp://status", status);
    });
    start_with_tokens(
        vault,
        requested_port,
        allow_write,
        runtime_file,
        vault_path,
        load_or_create_token("read")?,
        load_or_create_token("write")?,
        emit_status,
    )
    .await
}

async fn start_with_tokens(
    vault: Arc<Vault>,
    requested_port: Option<u16>,
    allow_write: bool,
    runtime_file: PathBuf,
    vault_path: PathBuf,
    read_token: String,
    write_token: String,
    emit_status: Arc<dyn Fn(McpStatus) + Send + Sync>,
) -> Result<McpServerRuntime, String> {
    let listener = TcpListener::bind(SocketAddr::new(
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        requested_port.unwrap_or(0),
    ))
    .await
    .map_err(|error| error.to_string())?;
    let port = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .port();
    let host = Arc::new(ToolHost::new(
        vault,
        Policy {
            allow_write,
            ..Policy::default()
        },
    ));
    let state = Arc::new(ServerState {
        emit_status,
        host,
        read_token: std::sync::RwLock::new(read_token),
        write_token: std::sync::RwLock::new(write_token),
        allow_write: AtomicBool::new(allow_write),
        port,
        active_requests: AtomicUsize::new(0),
        last_call_at: std::sync::RwLock::new(None),
    });
    state.refresh_runtime_file(&runtime_file)?;

    let cancellation = CancellationToken::new();
    let mcp_config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true)
        .with_max_request_body_bytes(256 * 1024)
        .with_cancellation_token(cancellation.clone())
        .with_allowed_hosts([format!("127.0.0.1:{port}"), format!("localhost:{port}")])
        .with_allowed_origins(["tauri://localhost", "http://tauri.localhost"])
        .enforce_origin_validation();
    let service_state = state.clone();
    let service = StreamableHttpService::new(
        move || {
            Ok(McpAdapter {
                state: service_state.clone(),
            })
        },
        Arc::new(LocalSessionManager::default()),
        mcp_config,
    );
    let router = Router::new()
        .route_service(MCP_PATH, service)
        .layer(middleware::from_fn_with_state(state.clone(), authenticate));
    let server_cancellation = cancellation.clone();
    tauri::async_runtime::spawn(async move {
        let _ = axum::serve(listener, router)
            .with_graceful_shutdown(server_cancellation.cancelled_owned())
            .await;
    });

    state.publish_status();
    Ok(McpServerRuntime {
        state,
        cancellation,
        runtime_file,
        vault_path,
    })
}

async fn authenticate(
    State(state): State<Arc<ServerState>>,
    mut request: Request,
    next: Next,
) -> Response {
    let expected_host = [
        format!("127.0.0.1:{}", state.port),
        format!("localhost:{}", state.port),
    ];
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok());
    if !host.is_some_and(|host| expected_host.iter().any(|allowed| allowed == host)) {
        return StatusCode::FORBIDDEN.into_response();
    }
    if let Some(origin) = request.headers().get(header::ORIGIN) {
        if !origin
            .to_str()
            .is_ok_and(|origin| matches!(origin, "tauri://localhost" | "http://tauri.localhost"))
        {
            return StatusCode::FORBIDDEN.into_response();
        }
    }
    let Some(token) = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let access = if state
        .read_token
        .read()
        .is_ok_and(|value| value.as_str() == token)
    {
        Access::ReadOnly
    } else if state
        .write_token
        .read()
        .is_ok_and(|value| value.as_str() == token)
    {
        Access::ReadWrite
    } else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    request.extensions_mut().insert(AuthenticatedAccess(access));
    state.active_requests.fetch_add(1, Ordering::AcqRel);
    *state.last_call_at.write().expect("MCP status lock") = Some(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
    );
    state.publish_status();
    let response = next.run(request).await;
    state.active_requests.fetch_sub(1, Ordering::AcqRel);
    state.publish_status();
    response
}

#[derive(Clone)]
struct McpAdapter {
    state: Arc<ServerState>,
}

fn access(ctx: &RequestContext<RoleServer>) -> Result<Access, ErrorData> {
    ctx.extensions
        .get::<http::request::Parts>()
        .and_then(|parts| parts.extensions.get::<AuthenticatedAccess>())
        .map(|access| access.0)
        .ok_or_else(|| ErrorData::invalid_request("unauthorized", None))
}

impl ServerHandler for McpAdapter {
    fn get_info(&self) -> ServerConfig {
        let mut info = ServerConfig::default();
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info
    }

    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        if request.is_some_and(|params| params.cursor.is_some()) {
            return Err(ErrorData::invalid_params("invalid_cursor", None));
        }
        let access = access(&ctx)?;
        Ok(ListToolsResult {
            tools: self
                .state
                .host
                .list_tools(access)
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
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let access = access(&ctx)?;
        let result = self
            .state
            .host
            .call(
                CallCtx {
                    access,
                    cancel: ctx.ct,
                    deadline: tokio::time::Instant::now() + Duration::from_secs(30),
                },
                &request.name,
                serde_json::Value::Object(request.arguments.unwrap_or_default()),
            )
            .await;
        let result = match result {
            Ok(value) => CallToolResult::structured(value),
            Err(error) => CallToolResult::error(vec![ContentBlock::text(error.to_string())]),
        };
        Ok(result.into())
    }
}

fn keychain_entry(name: &str) -> Result<Entry, String> {
    Entry::new(KEYCHAIN_SERVICE, name).map_err(|error| error.to_string())
}

fn load_or_create_token(name: &str) -> Result<String, String> {
    let entry = keychain_entry(name)?;
    match entry.get_password() {
        Ok(token) => Ok(token),
        Err(keyring::Error::NoEntry) => {
            let token = uuid::Uuid::new_v4().to_string();
            entry
                .set_password(&token)
                .map_err(|error| error.to_string())?;
            Ok(token)
        }
        Err(error) => Err(error.to_string()),
    }
}

fn write_runtime_file(path: &Path, runtime: &RuntimeFile) -> Result<(), String> {
    use std::io::Write;
    let parent = path.parent().ok_or("runtime path has no parent")?;
    std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let temporary = path.with_extension("json.tmp");
    let mut options = std::fs::OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temporary)
        .map_err(|error| error.to_string())?;
    file.write_all(&serde_json::to_vec(runtime).map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o600))
            .map_err(|error| error.to_string())?;
    }
    std::fs::rename(temporary, path).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::{
        model::{CallToolRequestParams, ClientConfig},
        service::ServiceExt,
        transport::{
            StreamableHttpClientTransport,
            streamable_http_client::StreamableHttpClientTransportConfig,
        },
    };
    use serde_json::json;

    #[tokio::test]
    async fn streamable_http_requires_auth_rejects_untrusted_origin_and_limits_read_token() {
        let vault_dir = tempfile::tempdir().expect("vault temp dir");
        let runtime_dir = tempfile::tempdir().expect("runtime temp dir");
        let runtime_path = runtime_dir.path().join("runtime.json");
        let vault = Arc::new(Vault::open(wiki_core::VaultConfig::new(vault_dir.path())).unwrap());
        let runtime = start_with_tokens(
            vault,
            None,
            true,
            runtime_path.clone(),
            vault_dir.path().to_path_buf(),
            "read-test-token".into(),
            "write-test-token".into(),
            Arc::new(|_| {}),
        )
        .await
        .unwrap();
        let info = runtime.info().unwrap();
        let stdio_config: serde_json::Value = serde_json::from_str(&info.stdio_config).unwrap();
        assert_eq!(
            stdio_config["mcpServers"]["wiki"]["args"],
            json!(["--vault", vault_dir.path().to_string_lossy()])
        );
        let uri = format!("http://127.0.0.1:{}/mcp", info.port);
        let client = reqwest::Client::new();

        let no_token = client
            .post(&uri)
            .json(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}))
            .send()
            .await
            .unwrap();
        assert_eq!(no_token.status(), StatusCode::UNAUTHORIZED);

        let query_token = client
            .post(format!("{uri}?token=read-test-token"))
            .json(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}))
            .send()
            .await
            .unwrap();
        assert_eq!(query_token.status(), StatusCode::UNAUTHORIZED);

        let bad_origin = client
            .post(&uri)
            .bearer_auth("read-test-token")
            .header(header::ORIGIN, "https://evil.example")
            .json(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}))
            .send()
            .await
            .unwrap();
        assert_eq!(bad_origin.status(), StatusCode::FORBIDDEN);

        let bad_host = client
            .post(&uri)
            .bearer_auth("read-test-token")
            .header(header::HOST, "evil.example")
            .json(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}))
            .send()
            .await
            .unwrap();
        assert_eq!(bad_host.status(), StatusCode::FORBIDDEN);

        let transport = StreamableHttpClientTransport::from_config(
            StreamableHttpClientTransportConfig::with_uri(uri).auth_header("read-test-token"),
        );
        let mcp = ClientConfig::default().serve(transport).await.unwrap();
        let tools = mcp.list_all_tools().await.unwrap();
        assert!(tools.iter().any(|tool| tool.name == "wiki_status"));
        assert!(!tools.iter().any(|tool| tool.name == "wiki_write_file"));
        let write = mcp
            .call_tool(
                CallToolRequestParams::new("wiki_write_file").with_arguments(
                    json!({"scope":"wiki","rel_path":"blocked.md","content":"no"})
                        .as_object()
                        .unwrap()
                        .clone(),
                ),
            )
            .await
            .unwrap();
        assert!(write.is_error.unwrap_or(false));
        mcp.cancel().await.unwrap();

        runtime.state.set_allow_write(false);
        let write_token_client = StreamableHttpClientTransport::from_config(
            StreamableHttpClientTransportConfig::with_uri(format!(
                "http://127.0.0.1:{}/mcp",
                info.port
            ))
            .auth_header("write-test-token"),
        );
        let read_only = ClientConfig::default()
            .serve(write_token_client)
            .await
            .unwrap();
        let tools = read_only.list_all_tools().await.unwrap();
        assert!(!tools.iter().any(|tool| tool.name == "wiki_write_file"));
        let write = read_only
            .call_tool(
                CallToolRequestParams::new("wiki_write_file").with_arguments(
                    json!({"scope":"wiki","rel_path":"blocked-by-setting.md","content":"no"})
                        .as_object()
                        .unwrap()
                        .clone(),
                ),
            )
            .await
            .unwrap();
        assert!(write.is_error.unwrap_or(false));
        read_only.cancel().await.unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&runtime_path)
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        runtime.shutdown();
        assert!(!runtime_path.exists());
    }
}
