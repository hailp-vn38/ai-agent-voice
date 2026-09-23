//! Typed, transport-free Device MCP vocabulary.

use std::collections::{HashMap, HashSet};

use serde_json::{Map, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct McpRequestId(pub u64);

#[derive(Clone, Debug, PartialEq)]
pub enum McpIncoming {
    Result {
        id: McpRequestId,
        result: Value,
    },
    Error {
        id: McpRequestId,
        code: i64,
        message: String,
    },
    Notification {
        method: String,
        params: Option<Value>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum McpOutgoing {
    Initialize {
        id: McpRequestId,
    },
    ToolsList {
        id: McpRequestId,
        cursor: Option<String>,
    },
    ToolsCall {
        id: McpRequestId,
        name: String,
        arguments: Map<String, Value>,
    },
}

impl McpOutgoing {
    pub fn payload(&self) -> Value {
        match self {
            Self::Initialize { id } => serde_json::json!({
                "jsonrpc": "2.0", "id": id.0, "method": "initialize",
                "params": {"protocolVersion": "2024-11-05", "capabilities": {},
                    "clientInfo": {"name": "voice-agent-server", "version": "0.1.0"}}
            }),
            Self::ToolsList { id, cursor } => {
                let mut request =
                    serde_json::json!({"jsonrpc":"2.0", "id":id.0, "method":"tools/list"});
                if let Some(cursor) = cursor {
                    request["params"] = serde_json::json!({"cursor": cursor});
                }
                request
            }
            Self::ToolsCall {
                id,
                name,
                arguments,
            } => serde_json::json!({
                "jsonrpc":"2.0", "id":id.0, "method":"tools/call",
                "params": {"name": name, "arguments": arguments}
            }),
        }
    }
}

pub fn parse_incoming(payload: Value) -> Option<McpIncoming> {
    if payload.get("jsonrpc")?.as_str()? != "2.0" {
        return None;
    }
    if let Some(method) = payload.get("method").and_then(Value::as_str) {
        return Some(McpIncoming::Notification {
            method: method.into(),
            params: payload.get("params").cloned(),
        });
    }
    let id = McpRequestId(payload.get("id")?.as_u64()?);
    if let Some(result) = payload.get("result") {
        return Some(McpIncoming::Result {
            id,
            result: result.clone(),
        });
    }
    let error = payload.get("error")?.as_object()?;
    Some(McpIncoming::Error {
        id,
        code: error.get("code").and_then(Value::as_i64)?,
        message: error.get("message").and_then(Value::as_str)?.to_owned(),
    })
}

#[derive(Clone, Debug, PartialEq)]
pub struct DiscoveredTool {
    pub original_name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LlmVisibleTool {
    pub llm_name: String,
    pub original_name: String,
    pub description: String,
    pub input_schema: Value,
}

pub fn sanitize_tool_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

pub fn is_dangerous_tool(name: &str) -> bool {
    matches!(
        name,
        "self.reboot" | "self.reset" | "self.factory_reset" | "self.upgrade_firmware"
    ) || name.starts_with("shell.")
        || name.starts_with("command.")
        || name.starts_with("exec.")
}

pub fn visible_tools(
    discovered: Vec<DiscoveredTool>,
    allowed: &HashSet<String>,
) -> Vec<LlmVisibleTool> {
    let mut candidate: Vec<_> = discovered
        .into_iter()
        .filter(|tool| {
            (allowed.is_empty() || allowed.contains(&tool.original_name))
                && !is_dangerous_tool(&tool.original_name)
        })
        .collect();
    let mut names = HashMap::<String, usize>::new();
    for tool in &candidate {
        *names
            .entry(sanitize_tool_name(&tool.original_name))
            .or_default() += 1;
    }
    candidate
        .drain(..)
        .filter_map(|tool| {
            let llm_name = sanitize_tool_name(&tool.original_name);
            (names[&llm_name] == 1).then(|| LlmVisibleTool {
                llm_name,
                original_name: tool.original_name,
                description: tool.description,
                input_schema: tool.input_schema,
            })
        })
        .collect()
}

pub fn parse_tools_page(result: &Value) -> Option<(Vec<DiscoveredTool>, Option<String>)> {
    let object = result.as_object()?;
    let tools = object
        .get("tools")?
        .as_array()?
        .iter()
        .map(|tool| {
            Some(DiscoveredTool {
                original_name: tool.get("name")?.as_str()?.to_owned(),
                description: tool
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                input_schema: tool.get("inputSchema")?.clone(),
            })
        })
        .collect::<Option<Vec<_>>>()?;
    let cursor = object
        .get("nextCursor")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    Some((tools, cursor))
}
