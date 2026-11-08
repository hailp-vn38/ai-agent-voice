use rmcp::{
    ErrorData, RoleServer, ServerHandler, ServiceExt,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ListToolsResult,
        PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
    },
    service::RequestContext,
    transport::stdio,
};
use std::{path::PathBuf, process::ExitCode, sync::Arc, time::Duration};
use wiki_core::{Vault, VaultConfig};
use wiki_mcp::{Access, CallCtx, Policy, ToolHost};

#[derive(Debug)]
struct Args {
    vault: PathBuf,
    allow_write: bool,
}

fn parse_args() -> Result<Option<Args>, String> {
    let mut args = std::env::args_os().skip(1);
    let mut vault = None;
    let mut allow_write = false;
    while let Some(argument) = args.next() {
        match argument.to_str() {
            Some("--vault") => {
                let path = args.next().ok_or("--vault requires a directory")?;
                vault = Some(PathBuf::from(path));
            }
            Some("--allow-write") => allow_write = true,
            Some("--help" | "-h") => return Ok(None),
            _ => return Err(format!("unknown argument: {}", argument.to_string_lossy())),
        }
    }
    let vault = vault.or_else(|| std::env::var_os("WIKI_VAULT_PATH").map(PathBuf::from));
    let Some(vault) = vault else {
        return Err("pass --vault <directory> or set WIKI_VAULT_PATH".into());
    };
    Ok(Some(Args { vault, allow_write }))
}

#[derive(Clone)]
struct StdioHandler {
    host: Arc<ToolHost>,
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
        if request.is_some_and(|params| params.cursor.is_some()) {
            return Err(ErrorData::invalid_params("invalid_cursor", None));
        }
        Ok(ListToolsResult {
            tools: self
                .host
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
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let result = self
            .host
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
            Err(error) => CallToolResult::error(vec![ContentBlock::text(error.to_string())]),
        };
        Ok(result.into())
    }
}

async fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    let vault = Arc::new(Vault::open(VaultConfig::new(args.vault))?);
    let writable = vault.is_writable();
    if !writable {
        eprintln!("wiki-mcp: vault is open read-only because another writer holds its lock");
    }
    let host = Arc::new(ToolHost::new(
        vault,
        Policy {
            allow_write: args.allow_write,
            ..Policy::default()
        },
    ));
    let service = StdioHandler { host };
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
                "Usage: wiki-mcp --vault <directory> [--allow-write]\n       WIKI_VAULT_PATH may replace --vault"
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
