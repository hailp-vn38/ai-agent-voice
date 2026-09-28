use super::*;
use crate::tools::device_mcp::{
    LlmVisibleTool, McpIncoming, McpOutgoing, parse_tools_page, visible_tools,
};

impl SessionActor {
    pub(super) fn begin_mcp_discovery(&mut self) {
        if !self.mcp.enabled || self.mcp.failed || self.mcp.ready {
            return;
        }
        let Some(id) = self.next_mcp_request_id() else {
            self.mcp.failed = true;
            return;
        };
        self.send_mcp(
            McpOutgoing::Initialize {
                id,
                vision: self.mcp.vision.clone(),
            },
            None,
            PendingMcpKind::Initialize,
            self.mcp.discovery_timeout,
        );
    }

    pub(super) fn on_mcp_message(&mut self, incoming: McpIncoming) {
        let (id, response) = match incoming {
            McpIncoming::Result { id, result } => (id, Ok(result)),
            McpIncoming::Error { id, .. } => (id, Err("device_tool_error")),
            McpIncoming::Notification { .. } => return,
        };
        let Some(pending) = self.mcp.pending.remove(&id) else {
            return;
        };
        if pending
            .generation
            .is_some_and(|generation| generation != self.generation)
        {
            return;
        }
        match pending.kind {
            PendingMcpKind::Initialize => match response {
                Ok(_) => self.request_tools_page(None),
                Err(_) => self.mcp_discovery_failed(),
            },
            PendingMcpKind::ToolsList => match response
                .and_then(|value| parse_tools_page(&value).ok_or("malformed_tools_list"))
            {
                Ok((tools, cursor)) => {
                    self.mcp.discovered.extend(tools);
                    if let Some(cursor) = cursor {
                        self.request_tools_page(Some(cursor));
                    } else {
                        self.mcp.visible = visible_tools(
                            std::mem::take(&mut self.mcp.discovered),
                            &self.mcp.allowed_tools,
                        );
                        self.mcp.ready = true;
                        let tools = self
                            .mcp
                            .visible
                            .iter()
                            .map(|tool| format!("{} -> {}", tool.llm_name, tool.original_name))
                            .collect::<Vec<_>>();
                        tracing::info!(
                            event = "mcp_tools_loaded",
                            tool_count = tools.len(),
                            tools = ?tools,
                            "Device MCP tools loaded into the session registry"
                        );
                    }
                }
                Err(_) => self.mcp_discovery_failed(),
            },
            PendingMcpKind::ToolCall { call } => {
                tracing::info!(
                    event = "mcp_tool_result_received",
                    "Device MCP tool result received"
                );
                self.complete_tool_call(call, response)
            }
        }
    }

    fn request_tools_page(&mut self, cursor: Option<String>) {
        let Some(id) = self.next_mcp_request_id() else {
            self.mcp_discovery_failed();
            return;
        };
        self.send_mcp(
            McpOutgoing::ToolsList { id, cursor },
            None,
            PendingMcpKind::ToolsList,
            self.mcp.discovery_timeout,
        );
    }

    fn mcp_discovery_failed(&mut self) {
        self.mcp.failed = true;
        self.mcp.ready = false;
        self.mcp.visible.clear();
        self.mcp
            .pending
            .retain(|_, pending| matches!(pending.kind, PendingMcpKind::ToolCall { .. }));
    }

    pub(super) fn dispatch_device_mcp_tool(&mut self, call: ToolCall, tool: LlmVisibleTool) {
        if !self.mcp.ready {
            self.complete_tool_call(call, Err("mcp_unavailable"));
            return;
        }
        let Some(arguments) = call.arguments.as_object().cloned() else {
            self.complete_tool_call(call, Err("invalid_arguments"));
            return;
        };
        let Some(id) = self.next_mcp_request_id() else {
            self.complete_tool_call(call, Err("request_id_exhausted"));
            return;
        };
        self.send_mcp(
            McpOutgoing::ToolsCall {
                id,
                name: tool.original_name,
                arguments,
            },
            Some(self.generation),
            PendingMcpKind::ToolCall { call },
            self.mcp.call_timeout,
        );
    }

    fn send_mcp(
        &mut self,
        outgoing: McpOutgoing,
        generation: Option<u64>,
        kind: PendingMcpKind,
        timeout: std::time::Duration,
    ) {
        let id = match &outgoing {
            McpOutgoing::Initialize { id, .. }
            | McpOutgoing::ToolsList { id, .. }
            | McpOutgoing::ToolsCall { id, .. } => *id,
        };
        let payload = serde_json::json!({"type":"mcp", "session_id": self.session_id, "payload": outgoing.payload()});
        let Ok(text) = serde_json::to_string(&payload) else {
            self.mcp_discovery_failed();
            return;
        };
        let sent = if generation.is_some() {
            self.send_turn_control(text).is_ok()
        } else {
            self.send_control(text).is_ok()
        };
        if sent {
            if matches!(&kind, PendingMcpKind::ToolCall { .. }) {
                tracing::info!(event = "mcp_tool_call_sent", "Device MCP tool call sent");
            }
            self.mcp.pending.insert(
                id,
                PendingMcpRequest {
                    generation,
                    deadline: Instant::now() + timeout,
                    kind,
                },
            );
        } else if generation.is_some() {
            self.fail_speech_delivery();
        } else {
            self.mcp_discovery_failed();
        }
    }

    fn next_mcp_request_id(&mut self) -> Option<McpRequestId> {
        let next = self.mcp.next_request_id.checked_add(1)?;
        self.mcp.next_request_id = next;
        Some(McpRequestId(next))
    }

    pub(super) fn expire_mcp_requests(&mut self) {
        let now = Instant::now();
        let expired = self
            .mcp
            .pending
            .iter()
            .filter_map(|(id, request)| (request.deadline <= now).then_some(*id))
            .collect::<Vec<_>>();
        for id in expired {
            let Some(request) = self.mcp.pending.remove(&id) else {
                continue;
            };
            match request.kind {
                PendingMcpKind::Initialize | PendingMcpKind::ToolsList => {
                    self.mcp_discovery_failed()
                }
                PendingMcpKind::ToolCall { call } => self.complete_tool_call(call, Err("timeout")),
            }
        }
    }

    /// Drop semantic ownership of in-flight tool calls after a turn is cancelled.
    /// The device may still execute a side effect, but its late response cannot restart the LLM.
    pub(super) fn cancel_pending_mcp_turn(&mut self) {
        self.mcp
            .pending
            .retain(|_, pending| pending.generation.is_none());
    }
}

pub(super) fn normalize_tool_result(result: serde_json::Value, max_chars: usize) -> String {
    let is_error = result
        .get("isError")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    use unicode_normalization::UnicodeNormalization;
    let content = result
        .get("content")
        .and_then(serde_json::Value::as_array)
        .map(|content| {
            content
                .iter()
                .filter_map(|item| item.get("text").and_then(serde_json::Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    let sanitized: String = content
        .nfc()
        .filter(|character| {
            matches!(character, '\n' | '\t')
                || (!character.is_control()
                    && !matches!(*character, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'))
        })
        .collect();
    let mut chars = sanitized.chars();
    let content: String = chars.by_ref().take(max_chars).collect();
    let truncated = chars.next().is_some();
    serde_json::json!({
        "ok": !is_error,
        "code": is_error.then_some("device_tool_error"),
        "content": content,
        "truncated": truncated,
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::normalize_tool_result;

    #[test]
    fn normalizer_joins_nfc_sanitizes_and_caps_after_sanitization() {
        let result = serde_json::json!({
            "content": [
                {"text": "e\u{301}\u{0000}\u{202e}x"},
                {"text": "\tcd\n"},
                {"image": "ignored"}
            ]
        });
        let normalized: serde_json::Value =
            serde_json::from_str(&normalize_tool_result(result, 4)).unwrap();
        assert_eq!(normalized["content"], "éx\n\t");
        assert_eq!(normalized["truncated"], true);
    }

    #[test]
    fn normalizer_does_not_mark_exact_cap_as_truncated() {
        let result = serde_json::json!({"content": [{"text": "abcd"}]});
        let normalized: serde_json::Value =
            serde_json::from_str(&normalize_tool_result(result, 4)).unwrap();
        assert_eq!(normalized["content"], "abcd");
        assert_eq!(normalized["truncated"], false);
    }
}
