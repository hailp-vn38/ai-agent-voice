use super::*;
use crate::config::McpResultDelivery;
use crate::tools::device_mcp::{McpIncoming, McpOutgoing, parse_tools_page, visible_tools};

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
            McpOutgoing::Initialize { id },
            None,
            PendingMcpKind::Initialize,
            self.mcp.discovery_timeout,
        );
    }

    pub(super) fn on_mcp_message(&mut self, incoming: McpIncoming) {
        let (id, response) = match incoming {
            McpIncoming::Result { id, result } => (id, Ok(result)),
            McpIncoming::Error { id, .. } => (id, Err("device_error")),
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

    pub(super) fn start_tool_batch(&mut self, calls: Vec<ToolCall>) {
        if !self.mcp.ready || calls.is_empty() {
            self.fail_speech_delivery();
            return;
        }
        self.mcp.batch = Some(ToolBatchState {
            generation: self.generation,
            calls,
            next: 0,
            results: Vec::new(),
        });
        self.dispatch_next_tool();
    }

    fn dispatch_next_tool(&mut self) {
        let next = self.mcp.batch.as_ref().and_then(|batch| {
            (batch.generation == self.generation)
                .then(|| batch.calls.get(batch.next).cloned())
                .flatten()
        });
        let Some(call) = next else {
            if let Some(batch) = self.mcp.batch.take() {
                self.commit_tool_exchange(&batch);
                match self.batch_result_delivery(&batch) {
                    McpResultDelivery::LlmThenTts => self.begin_tool_continuation(),
                    McpResultDelivery::DirectTts => {
                        if let Some(text) = self.direct_tts_text(&batch) {
                            self.begin_direct_tool_speech(text);
                        } else {
                            self.finish_tool_turn_without_speech();
                        }
                    }
                    McpResultDelivery::Silent => self.finish_tool_turn_without_speech(),
                }
            }
            return;
        };
        let Some(tool) = self
            .mcp
            .visible
            .iter()
            .find(|tool| tool.llm_name == call.name)
            .cloned()
        else {
            self.complete_tool_call(call, Err("unknown_tool"));
            return;
        };
        let Some(arguments) = call.arguments.as_object().cloned() else {
            self.complete_tool_call(call, Err("invalid_arguments"));
            return;
        };
        let Some(id) = self.next_mcp_request_id() else {
            self.complete_tool_call(call, Err("request_id_exhausted"));
            return;
        };
        if let Some(batch) = self.mcp.batch.as_mut() {
            batch.next += 1;
        }
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

    fn complete_tool_call(&mut self, call: ToolCall, result: Result<serde_json::Value, &str>) {
        let content = match result {
            Ok(value) => normalize_tool_result(value),
            Err(code) => serde_json::json!({"ok": false, "code": code}).to_string(),
        };
        if let Some(batch) = self.mcp.batch.as_mut() {
            batch.results.push(ChatMessage::ToolResult {
                tool_call_id: call.id,
                content,
            });
        }
        self.dispatch_next_tool();
    }

    fn send_mcp(
        &mut self,
        outgoing: McpOutgoing,
        generation: Option<u64>,
        kind: PendingMcpKind,
        timeout: std::time::Duration,
    ) {
        let id = match &outgoing {
            McpOutgoing::Initialize { id }
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
    pub(super) fn cancel_mcp_turn(&mut self) {
        self.mcp
            .pending
            .retain(|_, pending| pending.generation.is_none());
        self.mcp.batch = None;
        self.llm_round = None;
    }

    fn commit_tool_exchange(&mut self, batch: &ToolBatchState) {
        self.llm_messages.push(ChatMessage::AssistantToolCalls {
            calls: batch.calls.clone(),
        });
        self.llm_messages.extend(batch.results.clone());
    }

    fn batch_result_delivery(&self, batch: &ToolBatchState) -> McpResultDelivery {
        batch
            .calls
            .iter()
            .fold(McpResultDelivery::Silent, |selected, call| {
                let delivery = self
                    .mcp
                    .visible
                    .iter()
                    .find(|tool| tool.llm_name == call.name)
                    .and_then(|tool| self.mcp.tool_delivery.get(&tool.original_name))
                    .copied()
                    .unwrap_or(self.mcp.result_delivery);
                match (selected, delivery) {
                    (McpResultDelivery::LlmThenTts, _) | (_, McpResultDelivery::LlmThenTts) => {
                        McpResultDelivery::LlmThenTts
                    }
                    (McpResultDelivery::DirectTts, _) | (_, McpResultDelivery::DirectTts) => {
                        McpResultDelivery::DirectTts
                    }
                    _ => McpResultDelivery::Silent,
                }
            })
    }

    fn direct_tts_text(&self, batch: &ToolBatchState) -> Option<String> {
        let text = batch
            .results
            .iter()
            .filter_map(|result| match result {
                ChatMessage::ToolResult { content, .. } => {
                    serde_json::from_str::<serde_json::Value>(content)
                        .ok()
                        .filter(|result| result["ok"].as_bool() == Some(true))
                        .and_then(|result| {
                            result["content"].as_str().map(str::trim).map(str::to_owned)
                        })
                }
                _ => None,
            })
            .filter(|text| !text.is_empty() && !text.starts_with('{') && !text.starts_with('['))
            .collect::<Vec<_>>();
        (!text.is_empty()).then(|| text.join("\n"))
    }

    fn finish_tool_turn_without_speech(&mut self) {
        self.generated_response.clear();
        self.complete_recognition();
    }
}

fn normalize_tool_result(result: serde_json::Value) -> String {
    let is_error = result
        .get("isError")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let content = result
        .get("content")
        .and_then(serde_json::Value::as_array)
        .and_then(|content| {
            content
                .iter()
                .find_map(|item| item.get("text").and_then(serde_json::Value::as_str))
        })
        .unwrap_or_default();
    let content: String = content.chars().take(4096).collect();
    serde_json::json!({
        "ok": !is_error,
        "code": is_error.then_some("device_tool_error"),
        "content": content,
        "truncated": content.chars().count() == 4096,
    })
    .to_string()
}
