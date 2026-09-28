use super::*;
use crate::{
    config::McpResultDelivery,
    providers::llm::ToolDefinition,
    tools::{
        builtin::{BuiltinTool, EXIT_TOOL_NAME, exit_tool_definition, parse_exit_args},
        device_mcp::LlmVisibleTool,
    },
};

#[derive(Clone, Debug)]
enum ToolTarget {
    Builtin(BuiltinTool),
    DeviceMcp(LlmVisibleTool),
}

impl SessionActor {
    pub(super) fn available_llm_tools(&self) -> Vec<ToolDefinition> {
        let mut tools = vec![exit_tool_definition()];
        tools.extend(
            self.mcp
                .visible
                .iter()
                .filter(|tool| tool.llm_name != EXIT_TOOL_NAME)
                .map(|tool| ToolDefinition {
                    name: tool.llm_name.clone(),
                    description: tool.description.clone(),
                    parameters: tool.input_schema.clone(),
                }),
        );
        tools
    }

    fn resolve_tool(&self, name: &str) -> Option<ToolTarget> {
        if name == EXIT_TOOL_NAME {
            return Some(ToolTarget::Builtin(BuiltinTool::EndConversation));
        }
        self.mcp
            .visible
            .iter()
            .find(|tool| tool.llm_name == name)
            .cloned()
            .map(ToolTarget::DeviceMcp)
    }

    pub(super) fn start_tool_batch(&mut self, calls: Vec<ToolCall>) {
        if calls.is_empty() {
            self.fail_speech_delivery();
            return;
        }
        self.tool_batch = Some(ToolBatchState {
            generation: self.generation,
            calls,
            completed_calls: Vec::new(),
            next: 0,
            results: Vec::new(),
            direct_response: None,
        });
        self.dispatch_next_tool();
    }

    fn dispatch_next_tool(&mut self) {
        let next = self.tool_batch.as_ref().and_then(|batch| {
            (batch.generation == self.generation)
                .then(|| batch.calls.get(batch.next).cloned())
                .flatten()
        });
        let Some(call) = next else {
            self.finish_tool_batch();
            return;
        };
        if let Some(batch) = self.tool_batch.as_mut() {
            batch.next += 1;
        }
        match self.resolve_tool(&call.name) {
            Some(ToolTarget::Builtin(tool)) => self.execute_builtin_tool(call, tool),
            Some(ToolTarget::DeviceMcp(tool)) => self.dispatch_device_mcp_tool(call, tool),
            None => self.complete_tool_call(call, Err("unknown_tool")),
        }
    }

    fn execute_builtin_tool(&mut self, call: ToolCall, tool: BuiltinTool) {
        match tool {
            BuiltinTool::EndConversation => {
                let Ok(args) = parse_exit_args(&call.arguments) else {
                    self.complete_tool_call(call, Err("invalid_arguments"));
                    return;
                };
                let Some(turn_id) = self.current_turn_id() else {
                    self.complete_tool_call(call, Err("no_active_turn"));
                    return;
                };
                tracing::info!(
                    event = "builtin_tool_called",
                    tool = EXIT_TOOL_NAME,
                    turn_id = turn_id.get(),
                    "Builtin conversation exit tool called"
                );
                self.pending_session_action =
                    Some(PendingSessionAction::CloseAfterTurn { turn_id });
                tracing::info!(
                    event = "session_close_after_turn_armed",
                    turn_id = turn_id.get(),
                    "Session will close after normal turn completion"
                );
                self.complete_builtin_tool_call(
                    call,
                    serde_json::json!({
                        "ok": true,
                        "code": null,
                        "content": "conversation_exit_requested",
                        "truncated": false,
                    })
                    .to_string(),
                    args.say_goodbye,
                );
            }
        }
    }

    pub(super) fn complete_tool_call(
        &mut self,
        call: ToolCall,
        result: Result<serde_json::Value, &str>,
    ) {
        let content = match result {
            Ok(value) => {
                crate::session::actor::mcp::log_action_envelope_shape(&value, &call.name);
                if let Some(response) =
                    crate::session::actor::mcp::parse_xiaozhi_direct_response(&value)
                {
                    let content = crate::session::actor::mcp::normalize_tool_result(
                        serde_json::json!({"content":[{"text":response}]}),
                        self.max_tool_result_chars,
                    );
                    let direct_response = serde_json::from_str::<serde_json::Value>(&content)
                        .ok()
                        .and_then(|normalized| normalized["content"].as_str().map(str::to_owned))
                        .filter(|text| !text.trim().is_empty());
                    tracing::info!(
                        event = "mcp_action_response_detected",
                        tool = %call.name,
                        response_chars = response.chars().count(),
                        "MCP action response detected"
                    );
                    self.record_tool_call(call, content);
                    if let (Some(batch), Some(response)) =
                        (self.tool_batch.as_mut(), direct_response)
                    {
                        batch.direct_response.get_or_insert(response);
                    }
                    self.dispatch_next_tool();
                    return;
                }
                tracing::info!(
                    event = "mcp_action_response_not_detected",
                    tool = %call.name,
                    "MCP result remains on generic tool-result path"
                );
                crate::session::actor::mcp::normalize_tool_result(
                    crate::session::actor::mcp::redact_photo_data_from_tool_result(value),
                    self.max_tool_result_chars,
                )
            }
            Err(code) => tool_error_content(code),
        };
        self.record_tool_call(call, content);
        self.dispatch_next_tool();
    }

    fn complete_builtin_tool_call(&mut self, call: ToolCall, content: String, response: String) {
        self.record_tool_call(call, content);
        if let Some(batch) = self.tool_batch.as_mut() {
            batch.direct_response.get_or_insert(response);
        }
        self.dispatch_next_tool();
    }

    fn record_tool_call(&mut self, call: ToolCall, content: String) {
        if let Some(batch) = self.tool_batch.as_mut() {
            batch.completed_calls.push(call.clone());
            batch.results.push(ChatMessage::ToolResult {
                tool_call_id: call.id,
                content,
            });
        }
    }

    fn finish_tool_batch(&mut self) {
        let Some(batch) = self.tool_batch.take() else {
            return;
        };
        self.commit_tool_exchange(&batch);
        if let Some(goodbye) = batch.direct_response {
            self.begin_direct_tool_speech(goodbye);
            return;
        }
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

    pub(super) fn commit_tool_exchange(&mut self, batch: &ToolBatchState) {
        if batch.completed_calls.is_empty() {
            return;
        }
        let Some(turn_id) = self.current_turn_id() else {
            return;
        };
        if self.dialogue_history.append_completed_round(
            turn_id,
            batch.completed_calls.clone(),
            batch.results.clone(),
        ) {
            crate::session::prompt::append_completed_round(
                &mut self.llm_messages,
                batch.completed_calls.clone(),
                batch.results.clone(),
            );
        }
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

    pub(super) fn cancel_tool_turn(&mut self) {
        self.cancel_pending_mcp_turn();
        if let Some(batch) = self.tool_batch.take() {
            self.commit_tool_exchange(&batch);
        }
        self.llm_round = None;
    }
}

fn tool_error_content(code: &str) -> String {
    serde_json::json!({
        "ok": false,
        "code": code,
        "content": "",
        "truncated": false,
    })
    .to_string()
}
