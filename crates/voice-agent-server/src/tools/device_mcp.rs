//! Typed, transport-free Device MCP vocabulary.

use std::collections::{HashMap, HashSet};

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct McpRequestId(pub u64);

#[derive(Clone, PartialEq, Eq)]
pub struct VisionCapability {
    pub url: String,
    pub token: String,
}

impl std::fmt::Debug for VisionCapability {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VisionCapability")
            .field("url", &self.url)
            .field("token", &"[REDACTED]")
            .finish()
    }
}

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
        vision: Option<VisionCapability>,
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
            Self::Initialize { id, vision } => serde_json::json!({
                "jsonrpc": "2.0", "id": id.0, "method": "initialize",
                "params": {"protocolVersion": "2024-11-05", "capabilities": vision.as_ref().map(|vision| serde_json::json!({"vision":{"url":vision.url,"token":vision.token}})).unwrap_or_else(|| serde_json::json!({})),
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
    let candidate: Vec<_> = discovered
        .into_iter()
        .filter(|tool| {
            (allowed.is_empty() || allowed.contains(&tool.original_name))
                && !is_dangerous_tool(&tool.original_name)
        })
        .collect();
    to_visible(candidate)
}

/// Fingerprint of the parts that define a Device tool contract, so a description or schema change
/// makes a saved approval stale instead of silently widening it.
pub fn contract_fingerprint(tool: &DiscoveredTool) -> String {
    let mut hasher = Sha256::new();
    hasher.update(tool.original_name.as_bytes());
    hasher.update([0]);
    hasher.update(tool.description.as_bytes());
    hasher.update([0]);
    hasher.update(tool.input_schema.to_string().as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Review-filtered Device tools: only a discovered tool whose `original_name` was approved at this
/// exact fingerprint becomes visible, so an unreviewed tool is denied by default and an approved
/// name that now advertises a different contract is drift rather than a new right.
pub fn reviewed_tools(
    discovered: Vec<DiscoveredTool>,
    contracts: &HashMap<String, String>,
) -> (Vec<LlmVisibleTool>, bool) {
    let mut drift = false;
    let mut candidate = Vec::new();
    for tool in discovered {
        match contracts.get(&tool.original_name) {
            Some(fingerprint)
                if fingerprint == &contract_fingerprint(&tool)
                    && !is_dangerous_tool(&tool.original_name) =>
            {
                candidate.push(tool)
            }
            Some(_) => drift = true,
            None => {}
        }
    }
    (to_visible(candidate), drift)
}

fn to_visible(candidate: Vec<DiscoveredTool>) -> Vec<LlmVisibleTool> {
    let mut names = HashMap::<String, usize>::new();
    for tool in &candidate {
        *names
            .entry(sanitize_tool_name(&tool.original_name))
            .or_default() += 1;
    }
    candidate
        .into_iter()
        .filter_map(|tool| {
            let llm_name = sanitize_tool_name(&tool.original_name);
            (names[&llm_name] == 1).then_some(LlmVisibleTool {
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
