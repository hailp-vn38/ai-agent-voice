//! The sequential tool-round state machine and its history/speech hand-off.

use super::*;

use crate::{
    config::McpResultDelivery,
    providers::llm::{ChatMessage, ToolCall},
    tools::round::ToolRoundFailure,
};

impl SessionActor {
    pub(crate) fn with_tool_status(mut self, enabled: bool) -> Self {
        self.tool_status = enabled;
        self
    }

    /// Best-effort status for the current admitted turn. Never hold up a tool call for UI.
    fn emit_tool_status(&self, name: &str, state: &'static str) {
        if !self.tool_status
            || name.is_empty()
            || name.len() > 128
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"_.:-".contains(&byte))
        {
            return;
        }
        let Some(turn_id) = self.current_turn_id() else {
            return;
        };
        let payload = serde_json::json!({
            "type": "tool",
            "session_id": self.session_id,
            "turn_id": turn_id.get(),
            "name": name,
            "state": state,
        });
        if self.send_turn_control(payload.to_string()).is_err() {
            tracing::debug!(
                event = "tool_status_dropped",
                "Tool status could not enter control queue"
            );
        }
    }

    /// The one place a tool round begins.
    pub(in super::super) fn start_tool_batch(&mut self, calls: Vec<ToolCall>) {
        if calls.is_empty() {
            self.fail_speech_delivery();
            return;
        }
        // The application gate is checked before any per-round cap: shutdown means this turn
        // starts no work at all, rather than merely failing whichever limit it happens to hit.
        if !self.admission_gate.is_open() {
            tracing::warn!(
                event = "tool_round_rejected",
                code = ToolRoundFailure::ShuttingDown.code(),
                calls = calls.len(),
                "The application is shutting down; no tool round was executed"
            );
            self.terminalize_turn_failure(TurnFailure::Tool(ToolRoundFailure::ShuttingDown));
            return;
        }
        if calls.len() > self.tool_rounds.limits.max_calls_per_round {
            tracing::warn!(
                event = "tool_round_rejected",
                code = ToolRoundFailure::CallLimitExceeded.code(),
                calls = calls.len(),
                max_calls_per_round = self.tool_rounds.limits.max_calls_per_round,
                "LLM round asked for more tool calls than the configured cap allows; none were executed"
            );
            self.terminalize_turn_failure(TurnFailure::Tool(ToolRoundFailure::CallLimitExceeded));
            return;
        }
        tracing::info!(
            event = "tool_round_started",
            generation = self.generation,
            turn_id = self.current_turn_id().map(TurnId::get),
            calls = calls.len(),
            tool_round = self.tool_rounds.rounds + 1,
            "Tool round started"
        );
        let delivery = self.round_result_delivery(&calls);
        self.tool_batch = Some(ToolBatchState {
            generation: self.generation,
            calls,
            completed_calls: Vec::new(),
            next: 0,
            results: Vec::new(),
            direct_response: None,
            in_flight: None,
            delivery,
        });
        self.dispatch_next_tool();
    }

    /// The executor's only step function. Re-entry occurs only after a call terminalizes, so calls
    /// execute strictly in model order and only one can be in flight.
    pub(in super::super) fn dispatch_next_tool(&mut self) {
        let Some(batch) = self.tool_batch.as_ref() else {
            return;
        };
        if batch.generation != self.generation {
            self.cancel_tool_turn();
            return;
        }
        // A round already in flight at shutdown may commit its completed prefix, but it cannot
        // consume another execution slot after the admission gate closes.
        if !self.admission_gate.is_open() {
            tracing::warn!(
                event = "tool_round_rejected",
                code = ToolRoundFailure::ShuttingDown.code(),
                generation = self.generation,
                turn_id = self.current_turn_id().map(TurnId::get),
                "The application is shutting down; the rest of this tool round was not executed"
            );
            self.terminalize_tool_round(ToolRoundFailure::ShuttingDown);
            return;
        }
        let Some(call) = batch.calls.get(batch.next).cloned() else {
            self.finish_tool_batch();
            return;
        };
        if let Some(batch) = self.tool_batch.as_mut() {
            batch.next += 1;
        }
        let Some(budget) = self.tool_rounds.budget_for_next_call() else {
            self.terminalize_tool_round(ToolRoundFailure::ExecutionBudgetExceeded);
            return;
        };
        let Some(target) = self.resolve_tool(&call.name) else {
            self.complete_tool_call(call, Err("unknown_tool"));
            return;
        };
        self.emit_tool_status(&call.name, "started");
        match target {
            ToolTarget::SystemMcp(system) => self.dispatch_system_tool(call, system, budget),
            ToolTarget::Builtin(tool) => self.execute_builtin_tool(call, tool),
            ToolTarget::DeviceMcp(tool) => self.dispatch_device_mcp_tool(call, tool, budget),
            ToolTarget::ExternalMcp(server, tool) => {
                self.dispatch_external_tool(call, &server, &tool, budget)
            }
        }
    }

    pub(in super::super) fn terminalize_tool_round(&mut self, failure: ToolRoundFailure) {
        self.job_pending = None;
        warn!(code = failure.code(), "tool round terminalized the turn");
        if let Some(batch) = self.tool_batch.take() {
            self.commit_tool_exchange(&batch);
        }
        self.llm_round = None;
        self.terminalize_turn_failure(TurnFailure::Tool(failure));
    }

    pub(in super::super) fn complete_tool_call(
        &mut self,
        call: ToolCall,
        result: Result<serde_json::Value, &str>,
    ) {
        let succeeded = result.as_ref().is_ok_and(|value| {
            value.get("isError").and_then(serde_json::Value::as_bool) != Some(true)
        });
        if crate::tools::builtin::is_task_suite_tool_name(&call.name)
            && !crate::tools::builtin::is_legacy_reminder_tool_name(&call.name)
        {
            let value = result.unwrap_or_else(crate::services::task_flow::voice_rejection);
            self.record_tool_call(call, value.to_string(), succeeded);
            self.dispatch_next_tool();
            return;
        }
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
                    self.record_tool_call(call, content, succeeded);
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
        self.record_tool_call(call, content, succeeded);
        self.dispatch_next_tool();
    }

    pub(in super::super) fn complete_builtin_tool_call(
        &mut self,
        call: ToolCall,
        content: String,
        response: String,
    ) {
        self.record_tool_call(call, content, true);
        if let Some(batch) = self.tool_batch.as_mut() {
            batch.direct_response.get_or_insert(response);
        }
        self.dispatch_next_tool();
    }

    pub(in super::super) fn record_tool_call(
        &mut self,
        call: ToolCall,
        content: String,
        succeeded: bool,
    ) {
        let name = call.name.clone();
        let Some(batch) = self.tool_batch.as_mut() else {
            return;
        };
        batch.completed_calls.push(call.clone());
        batch.results.push(ChatMessage::ToolResult {
            tool_call_id: call.id,
            content,
        });
        if self.resolve_tool(&name).is_some() {
            self.emit_tool_status(&name, if succeeded { "completed" } else { "failed" });
        }
    }

    fn finish_tool_batch(&mut self) {
        let Some(mut batch) = self.tool_batch.take() else {
            return;
        };
        self.commit_tool_exchange(&batch);
        let requires_job_receipt = batch.calls.iter().any(|call| {
            crate::tools::builtin::is_task_suite_tool_name(&call.name)
                || crate::tools::builtin::is_reminder_tool_name(&call.name)
        });
        if !requires_job_receipt && let Some(goodbye) = batch.direct_response.take() {
            self.begin_direct_tool_speech(goodbye);
            return;
        }
        match batch.delivery {
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

    pub(in super::super) fn commit_tool_exchange(&mut self, batch: &ToolBatchState) {
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

    pub(in super::super) fn cancel_tool_turn(&mut self) {
        self.job_pending = None;
        self.template_prepare = None;
        // A Normal writer outcome has sealed a managed boundary; later abort cancels
        // only new interaction/preparation, not that accepted boundary.
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
