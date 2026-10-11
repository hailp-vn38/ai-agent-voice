//! Transport-independent System MCP authority, validation and safe read dispatch.
mod catalog;
mod discovery;
mod proposals;

use crate::{
    database::{agents::get_agent_by, mcp_servers::mcp_by},
    providers::llm::ToolDefinition,
    services::{system_metrics::SystemMetrics, system_status::SystemStatus},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy, Debug, Default, serde::Serialize, Deserialize, sqlx::FromRow)]
#[serde(deny_unknown_fields)]
pub(crate) struct SystemToolGrants {
    pub system_read: bool,
    pub mcp_read: bool,
    #[serde(default)]
    pub mcp_discover: bool,
    #[serde(default)]
    pub mcp_manage_request: bool,
}

// Not deserializable: only the authenticated admission boundary constructs this identity.
#[derive(Clone)]
pub(crate) enum SystemCaller {
    WebAssistant {
        device_db_id: i64,
        agent_id: i64,
    },
    Remote {
        subject: String,
        jti: String,
        expires_at: i64,
    },
}

#[derive(Clone)]
pub(crate) struct SystemToolContext {
    pub caller: SystemCaller,
    pub grants: SystemToolGrants,
    pub revoked: Arc<CancellationToken>,
    pub cancellation: CancellationToken,
    pub deadline: Instant,
}

pub(crate) struct SystemMcp {
    enabled: bool,
    status: SystemStatus,
    metrics: Arc<SystemMetrics>,
    max_result_chars: usize,
    discovery: Option<(
        Arc<crate::tools::external_mcp::ExternalMcpManager>,
        Arc<dyn crate::database::secrets::SecretResolver>,
    )>,
    pending: proposals::PendingRequests,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum SystemToolError {
    #[error("system_tool_denied")]
    Denied,
    #[error("invalid_arguments")]
    InvalidArguments,
    #[error("system_tool_unavailable")]
    Unavailable,
    #[error("resource_not_found")]
    NotFound,
    #[error("system_tool_cancelled")]
    Cancelled,
    #[error("system_tool_timeout")]
    Timeout,
    #[error("tool_result_too_large")]
    ResultTooLarge,
}

impl From<sqlx::Error> for SystemToolError {
    fn from(error: sqlx::Error) -> Self {
        match error {
            sqlx::Error::RowNotFound => Self::NotFound,
            _ => Self::Unavailable,
        }
    }
}

impl SystemMcp {
    pub(crate) fn new(
        enabled: bool,
        status: SystemStatus,
        metrics: Arc<SystemMetrics>,
        max_result_chars: usize,
    ) -> Self {
        Self {
            enabled,
            status,
            metrics,
            max_result_chars,
            discovery: None,
            pending: proposals::PendingRequests::default(),
        }
    }

    pub(crate) fn with_discovery(
        mut self,
        manager: Option<Arc<crate::tools::external_mcp::ExternalMcpManager>>,
        secrets: Arc<dyn crate::database::secrets::SecretResolver>,
    ) -> Self {
        self.discovery = manager.map(|manager| (manager, secrets));
        self
    }

    pub(crate) fn list_tools(&self, ctx: &SystemToolContext) -> Vec<ToolDefinition> {
        if !self.enabled || ctx.revoked.is_cancelled() || !self.status.lifecycle.gate().is_open() {
            return Vec::new();
        }
        catalog::catalog(ctx.grants)
    }

    pub(crate) async fn call_tool(
        &self,
        ctx: &SystemToolContext,
        name: &str,
        arguments: Value,
    ) -> Result<Value, SystemToolError> {
        if !self.list_tools(ctx).iter().any(|tool| tool.name == name) {
            return Err(SystemToolError::Denied);
        }
        if Instant::now() >= ctx.deadline {
            return Err(SystemToolError::Timeout);
        }
        if arguments.to_string().len() > 4096 {
            return Err(SystemToolError::InvalidArguments);
        }
        tokio::select! {
            biased;
            () = ctx.revoked.cancelled() => Err(SystemToolError::Denied),
            () = ctx.cancellation.cancelled() => Err(SystemToolError::Cancelled),
            () = self.status.lifecycle.stopping().cancelled() => Err(SystemToolError::Denied),
            result = tokio::time::timeout_at(ctx.deadline, self.execute(ctx, name, arguments)) => {
                let value = result.map_err(|_| SystemToolError::Timeout)??;
                if ctx.revoked.is_cancelled() || !self.status.lifecycle.gate().is_open() {
                    return Err(SystemToolError::Denied);
                }
                if value.to_string().chars().count() > self.max_result_chars {
                    return Err(SystemToolError::ResultTooLarge);
                }
                Ok(value)
            }
        }
    }

    async fn execute(
        &self,
        ctx: &SystemToolContext,
        name: &str,
        arguments: Value,
    ) -> Result<Value, SystemToolError> {
        let db = self
            .status
            .database
            .as_ref()
            .ok_or(SystemToolError::Unavailable)?;
        if name == "mcp_discover_tools" {
            return self.discover(ctx, arguments).await;
        }
        let _publication = db.tool_security.publication.read().await;
        self.authorize(ctx, name).await?;
        if name.starts_with("mcp_request_") {
            return self.propose(ctx, name, arguments);
        }
        match name {
            "system_get_status" => {
                parse::<Empty>(arguments)?;
                Ok(self.status.summary().await?)
            }
            "system_get_resources" => {
                parse::<Empty>(arguments)?;
                serde_json::to_value(self.metrics.latest().ok_or(SystemToolError::Unavailable)?)
                    .map_err(|_| SystemToolError::Unavailable)
            }
            "mcp_list_servers" => {
                let args: Servers = parse(arguments)?;
                validate_page(args.page, args.page_size)?;
                let items: Vec<_> = db
                    .list_mcp_servers(args.page, args.page_size)
                    .await?
                    .iter()
                    .map(server_metadata)
                    .collect();
                Ok(json!({"items":items,"page":args.page,"page_size":args.page_size}))
            }
            "mcp_get_server" => {
                let args: Server = parse(arguments)?;
                validate_key(&args.server_key)?;
                Ok(server_metadata(&mcp_by(db, &args.server_key).await?))
            }
            "mcp_list_tools" => {
                let args: Tools = parse(arguments)?;
                validate_page(args.page, 20)?;
                if let Some(key) = &args.server_key {
                    validate_key(key)?;
                }
                let agent_id = match args.agent_key {
                    Some(key) => {
                        validate_key(&key)?;
                        get_agent_by(db, &key).await?.id
                    }
                    None => match ctx.caller {
                        SystemCaller::WebAssistant { agent_id, .. } => agent_id,
                        SystemCaller::Remote { .. } => {
                            return Err(SystemToolError::InvalidArguments);
                        }
                    },
                };
                Ok(
                    json!({"items":db.system_mcp_tools(agent_id, args.server_key.as_deref(), args.page).await?,"page":args.page,"page_size":20}),
                )
            }
            "mcp_get_agent_bindings" => {
                let args: Agent = parse(arguments)?;
                validate_key(&args.agent_key)?;
                let agent = get_agent_by(db, &args.agent_key).await?;
                let items = db.system_mcp_bindings(agent.id).await?;
                if items.len() > 100 {
                    return Err(SystemToolError::ResultTooLarge);
                }
                Ok(
                    json!({"agent_key":agent.key,"enabled":agent.enabled != 0,"revision":agent.revision,"items":items}),
                )
            }
            _ => Err(SystemToolError::Denied),
        }
    }
}

fn server_metadata(row: &crate::database::mcp_servers::McpServerRow) -> Value {
    json!({"key":row.key,"name":row.name,"enabled":row.enabled != 0,"transport":"streamable_http","revision":row.revision})
}

fn parse<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, SystemToolError> {
    if !value.is_object() {
        return Err(SystemToolError::InvalidArguments);
    }
    serde_json::from_value(value).map_err(|_| SystemToolError::InvalidArguments)
}
fn validate_key(key: &str) -> Result<(), SystemToolError> {
    if key.len() > 64
        || !key.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        || !key
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    {
        return Err(SystemToolError::InvalidArguments);
    }
    Ok(())
}
fn validate_page(page: u32, size: u32) -> Result<(), SystemToolError> {
    if !(1..=10000).contains(&page) || !(1..=100).contains(&size) {
        return Err(SystemToolError::InvalidArguments);
    }
    Ok(())
}
fn first_page() -> u32 {
    1
}
fn default_size() -> u32 {
    20
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Servers {
    #[serde(default = "first_page")]
    page: u32,
    #[serde(default = "default_size")]
    page_size: u32,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Server {
    server_key: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Agent {
    agent_key: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Tools {
    #[serde(default, deserialize_with = "optional_key")]
    agent_key: Option<String>,
    #[serde(default, deserialize_with = "optional_key")]
    server_key: Option<String>,
    #[serde(default = "first_page")]
    page: u32,
}

#[derive(Clone)]
pub(crate) struct SessionSystemMcp {
    pub core: Arc<SystemMcp>,
    pub context: SystemToolContext,
    pub tools: Vec<ToolDefinition>,
}

fn optional_key<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    String::deserialize(deserializer).map(Some)
}

#[cfg(test)]
pub(crate) mod tests;

#[cfg(test)]
mod discovery_tests;

#[cfg(test)]
mod authority_tests;
