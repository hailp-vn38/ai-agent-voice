use super::*;
use std::sync::Arc;

use crate::{
    config::McpResultDelivery,
    providers::{ResolvedAgentRuntimes, llm::ToolDefinition},
    tools::{
        builtin::{
            BuiltinTool, EXIT_TOOL_NAME, SWITCH_TEMPLATE_TOOL_NAME, exit_tool_definition,
            parse_exit_args, parse_switch_template_args, switch_template_tool_definition,
        },
        device_mcp::LlmVisibleTool,
        external_mcp::{ResolvedExternalMcp, ResolvedExternalTool},
        round::external_tool_result,
    },
};

/// A builtin tool must not be shadowed by, or shadow, a Device MCP tool. Sanitized MCP names can
/// never contain a dot, so the dotted server action is unambiguous while the exit tool is filtered.
pub(super) fn is_builtin_tool_name(name: &str) -> bool {
    name == EXIT_TOOL_NAME || name == SWITCH_TEMPLATE_TOOL_NAME
}

/// Where one ToolCall goes, decided from the name the model used and nothing else.
///
/// Every arm is a capability this session was actually admitted with, so resolving to one is a
/// statement about this session's immutable catalog rather than about what exists anywhere.
#[derive(Clone, Debug)]
enum ToolTarget {
    Builtin(BuiltinTool),
    DeviceMcp(LlmVisibleTool),
    ExternalMcp(ResolvedExternalMcp, ResolvedExternalTool),
}

impl SessionActor {
    /// Everything this session may call, in the order the model will see it.
    ///
    /// The order is the admission order, not a ranking: the catalog is immutable, so what this
    /// returns is a property of the session rather than a snapshot of anything that can change.
    pub(super) fn available_llm_tools(&self) -> Vec<ToolDefinition> {
        let mut tools = vec![exit_tool_definition()];
        // A session whose admission catalog is empty never offers a capability it cannot honor.
        if !self.switch_catalog.is_empty() {
            tools.push(switch_template_tool_definition(
                &self.switch_catalog.template_keys(),
            ));
        }
        tools.extend(
            self.mcp
                .visible
                .iter()
                .filter(|tool| !is_builtin_tool_name(&tool.llm_name))
                .map(|tool| ToolDefinition {
                    name: tool.llm_name.clone(),
                    description: tool.description.clone(),
                    parameters: tool.input_schema.clone(),
                }),
        );
        tools.extend(self.external_mcp.servers().iter().flat_map(|server| {
            server.tools.iter().map(|tool| ToolDefinition {
                name: tool.llm_name.clone(),
                description: tool.description.clone(),
                parameters: tool.input_schema.clone(),
            })
        }));
        tools
    }

    /// Whether this session holds a tool that can change the final answer.
    ///
    /// A round that might call one stays buffered: its prose must not reach SpeechOutput before
    /// the tools have had their say.  The builtin actions are excluded on purpose, so an ordinary
    /// answer keeps streaming to speech merely because those are available.
    pub(super) fn offers_answer_changing_tools(&self) -> bool {
        !self.mcp.visible.is_empty() || !self.external_mcp.is_empty()
    }

    fn resolve_tool(&self, name: &str) -> Option<ToolTarget> {
        if name == EXIT_TOOL_NAME {
            return Some(ToolTarget::Builtin(BuiltinTool::EndConversation));
        }
        if name == SWITCH_TEMPLATE_TOOL_NAME {
            return Some(ToolTarget::Builtin(BuiltinTool::SwitchTemplate));
        }
        // Device MCP is consulted first because an External MCP name is namespaced and can never
        // collide with a sanitized Device MCP name, and routing through the origin rather than
        // through the name is what keeps the two apart.
        if let Some(tool) = self
            .mcp
            .visible
            .iter()
            .find(|tool| tool.llm_name == name)
            .cloned()
        {
            return Some(ToolTarget::DeviceMcp(tool));
        }
        self.external_mcp
            .find(name)
            .map(|(server, tool)| ToolTarget::ExternalMcp(server.clone(), tool.clone()))
    }

    /// The one place a tool round begins.
    ///
    /// The whole round is validated before call one, so a model that asks for more calls than the
    /// deployment allows has none of them executed: there is no partial round to reason about
    /// afterwards, and no ToolResult to invent for a call that never ran.
    pub(super) fn start_tool_batch(&mut self, calls: Vec<ToolCall>) {
        if calls.is_empty() {
            self.fail_speech_delivery();
            return;
        }
        // The application gate is asked before the round's own caps, because a closed gate means
        // no work of any kind starts rather than this round happening to exceed a limit.
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

    /// Starts the next call of the round, or ends the round.
    ///
    /// This is the executor's only step function, and it is what makes the round strictly
    /// sequential: it is re-entered only from a call's terminal outcome, so a second call cannot
    /// begin until the previous one has produced its result, and model order is therefore
    /// execution order is result order.
    fn dispatch_next_tool(&mut self) {
        let Some(batch) = self.tool_batch.as_ref() else {
            return;
        };
        if batch.generation != self.generation {
            // The turn that owned this round is gone.  Its completed calls keep their history, and
            // nothing else in the round starts — not even another Device MCP request.
            self.cancel_tool_turn();
            return;
        }
        // Asked per call rather than only per round, so a round that was already in flight when
        // shutdown began cannot use the next slot in its own budget to start another call.
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
        // Whatever is left of the turn's execution budget bounds this call.  A turn that has spent
        // it starts nothing further: the alternative is a side effect whose result could never be
        // reported back to the model.
        let Some(budget) = self.tool_rounds.budget_for_next_call() else {
            self.terminalize_tool_round(ToolRoundFailure::ExecutionBudgetExceeded);
            return;
        };
        match self.resolve_tool(&call.name) {
            Some(ToolTarget::Builtin(tool)) => self.execute_builtin_tool(call, tool),
            Some(ToolTarget::DeviceMcp(tool)) => self.dispatch_device_mcp_tool(call, tool, budget),
            Some(ToolTarget::ExternalMcp(server, tool)) => {
                self.dispatch_external_tool(call, &server, &tool, budget)
            }
            None => self.complete_tool_call(call, Err("unknown_tool")),
        }
    }

    /// Starts the single outbound attempt this External Tool Call is allowed to make.
    ///
    /// The request leaves the process on a task of its own, so the session's state machine is never
    /// blocked on a network call, and the completion comes back through this session's own bounded
    /// mailbox.  Cancellation is that task's own arm rather than something the session discovers
    /// afterwards, so an interrupted turn drops the in-flight request instead of waiting it out.
    ///
    /// There is no retry anywhere in this path: a side effect may already have happened remotely
    /// before a timeout or a dropped connection, so a second attempt is not a recovery strategy.
    fn dispatch_external_tool(
        &mut self,
        call: ToolCall,
        server: &ResolvedExternalMcp,
        tool: &ResolvedExternalTool,
        budget: std::time::Duration,
    ) {
        let Some(turn) = self.turn.as_ref() else {
            self.complete_external_tool_call(call, Err(&ExternalMcpError::ToolUnavailable));
            return;
        };
        let (turn_id, generation) = (turn.turn_id, turn.generation);
        let cancellation = turn.cancellation.clone();
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            // Without a runtime no request can leave the process at all, which is the same
            // observable outcome as a permit that never became available: nothing was sent.
            self.complete_external_tool_call(call, Err(&ExternalMcpError::ToolUnavailable));
            return;
        };
        let arguments = call.arguments.clone();
        let client = Arc::clone(&server.client);
        let limiter = Arc::clone(&server.limiter);
        let call_id = call.id.clone();
        let completions = self.external_calls_tx.clone();
        let tool = tool.clone();
        let server_key = server.server_key.clone();
        tracing::info!(
            event = "external_tool_call_sent",
            generation = self.generation,
            turn_id = turn_id.get(),
            server_key = %server_key,
            "External MCP tool call sent"
        );
        runtime.spawn(async move {
            let outcome = tokio::select! {
                biased;
                // Cancellation first: a turn that has been interrupted has no use for whatever
                // this call answers, so nothing is sent and nothing is waited for.
                () = cancellation.cancelled() => {
                    client.telemetry().call_cancelled(&server_key);
                    return;
                }
                outcome = client.call_tool(&limiter, &tool, &arguments, budget) => outcome,
            };
            let completion = ExternalCallCompletion {
                turn_id,
                generation,
                call_id,
                server_key: server_key.clone(),
                outcome,
            };
            // A mailbox that cannot take this is the same observable situation as a round that is
            // already gone: nobody may act on the result, so it is counted as late rather than
            // dropped in silence.  A closed mailbox means the session itself is gone, which needs
            // no result and no retry at all.
            if completions.try_send(completion).is_err() {
                client.telemetry().late_response_discarded(&server_key);
            }
        });
        if let Some(batch) = self.tool_batch.as_mut() {
            batch.in_flight = Some(InFlightExternalCall {
                call,
                turn_id,
                generation,
            });
        }
    }

    /// Applies one External Tool Call's terminal outcome, then continues the round.
    ///
    /// The caller has already established that this completion belongs to the call in flight, so
    /// the slot is simply released here: exactly one call is outstanding at a time, and a slot that
    /// survived would mean a second call could start beside it.
    fn complete_external_tool_call(
        &mut self,
        call: ToolCall,
        outcome: Result<&crate::tools::external_mcp::ExternalToolOutcome, &ExternalMcpError>,
    ) {
        if let Some(batch) = self.tool_batch.as_mut() {
            batch.in_flight = None;
        }
        let content = external_tool_result(outcome.map_err(|error| *error));
        tracing::info!(
            event = "external_tool_result_received",
            generation = self.generation,
            turn_id = self.current_turn_id().map(TurnId::get),
            ok = outcome.is_ok(),
            "External MCP tool result received"
        );
        self.record_tool_call(call, content);
        self.dispatch_next_tool();
    }

    /// Applies the completions of External Tool Calls this session started.
    ///
    /// A completion whose turn, generation or call no longer matches the one in flight is a late
    /// response: it is counted and dropped, so it can produce no ToolResult, no continuation, no
    /// archive entry and no speech.
    pub(super) fn drain_external_call_completions(&mut self) {
        while let Ok(completion) = self.external_calls.try_recv() {
            let in_flight = self.tool_batch.as_ref().and_then(|batch| {
                batch.in_flight.as_ref().filter(|in_flight| {
                    in_flight.call.id == completion.call_id
                        && in_flight.turn_id == completion.turn_id
                        && in_flight.generation == completion.generation
                })
            });
            let Some(in_flight) = in_flight.map(|in_flight| in_flight.call.clone()) else {
                tracing::info!(
                    event = "external_tool_late_response_discarded",
                    generation = completion.generation,
                    turn_id = completion.turn_id.get(),
                    server_key = %completion.server_key,
                    "External MCP tool result arrived after its round ended and was discarded"
                );
                if let Some(telemetry) = self.external_telemetry(&completion.server_key) {
                    telemetry.late_response_discarded(&completion.server_key);
                }
                continue;
            };
            self.complete_external_tool_call(in_flight, completion.outcome.as_ref());
        }
    }

    /// The process-owned sink for one admitted server.
    ///
    /// The catalog is immutable, so a server a session once called is still there afterwards; this
    /// is how a late response is still counted against the server that produced it.
    fn external_telemetry(&self, server_key: &str) -> Option<Arc<dyn crate::telemetry::Telemetry>> {
        self.external_mcp
            .servers()
            .iter()
            .find(|server| server.server_key == server_key)
            .map(|server| Arc::clone(server.client.telemetry()))
    }

    /// Ends the turn for a cap or an exhausted execution budget.
    ///
    /// The completed prefix is committed first, because those calls did finish and a completed
    /// ToolResult must never be discarded.  Then the turn ends: no continuation, no synthetic
    /// result and no sentinel `tool_call_id` standing in for a call that never completed.
    pub(super) fn terminalize_tool_round(&mut self, failure: ToolRoundFailure) {
        warn!(code = failure.code(), "tool round terminalized the turn");
        if let Some(batch) = self.tool_batch.take() {
            self.commit_tool_exchange(&batch);
        }
        self.llm_round = None;
        self.terminalize_turn_failure(TurnFailure::Tool(failure));
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
                self.pending_actions.close_after_turn = Some(turn_id);
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
            BuiltinTool::SwitchTemplate => self.execute_template_switch(call),
        }
    }

    /// Schedules a Template the session admitted at its own admission.  Nothing here reads
    /// configuration: the catalog is an immutable snapshot, so a candidate that was never admitted
    /// — or was excluded at admission — simply does not resolve, and the model gets one coarse
    /// error that says nothing about which Templates exist.
    fn execute_template_switch(&mut self, call: ToolCall) {
        let Ok(args) = parse_switch_template_args(&call.arguments) else {
            self.complete_tool_call(call, Err("invalid_arguments"));
            return;
        };
        let Some(turn_id) = self.current_turn_id() else {
            self.complete_tool_call(call, Err("no_active_turn"));
            return;
        };
        if self.switch_catalog.find(&args.template).is_none() {
            tracing::warn!(
                event = "session_profile_switch_rejected",
                reason = "candidate_not_admitted",
                "A template switch named a candidate this session never admitted"
            );
            self.complete_tool_call(call, Err("template_switch_unavailable"));
            return;
        }
        tracing::info!(
            event = "builtin_tool_called",
            tool = SWITCH_TEMPLATE_TOOL_NAME,
            template_key = %args.template,
            turn_id = turn_id.get(),
            "Builtin template switch tool called"
        );
        self.pending_actions.switch_template_after_turn = Some(PendingTemplateSwitch {
            turn_id,
            template_key: args.template.clone(),
        });
        self.complete_tool_call(
            call,
            Ok(serde_json::json!({
                "ok": true,
                "code": null,
                "content": "template_switch_scheduled",
                "truncated": false,
            })),
        );
    }

    /// The only place a Template change becomes effective.  Prompt, language, providers and the
    /// already-loaded runtime handles are replaced together, then the Session Profile Revision
    /// advances; a candidate that no longer resolves leaves the active profile untouched.
    pub(super) fn apply_template_switch(&mut self, template_key: &str) {
        let Some(candidate) = self.switch_catalog.find(template_key).cloned() else {
            tracing::warn!(
                event = "session_profile_switch_dropped",
                reason = "candidate_not_admitted",
                "A scheduled template switch no longer names an admitted candidate"
            );
            return;
        };
        if let Err(error) = install_candidate_runtimes(self, &candidate.runtimes) {
            tracing::warn!(
                event = "session_profile_switch_failed",
                reason = "runtime_install_failed",
                %error,
                "The admitted candidate's already-loaded runtime could not be installed"
            );
            return;
        }
        let previous_revision = self.profile.revision;
        self.profile.switched_to(&candidate);
        tracing::info!(
            event = "session_profile_switched",
            template_key = %candidate.template_key,
            previous_revision,
            revision = self.profile.revision,
            "Session profile switched at a turn boundary"
        );
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

    /// The one place a completed call joins its round.
    ///
    /// Calls and results are pushed together, so `results[i]` always belongs to `calls[i]` and the
    /// round can only be handed to history and to the continuation once the two are the same
    /// length.  A call that has not completed is not here at all, which is what keeps a
    /// continuation from ever seeing an `AssistantToolCall` without its `ToolResult`.
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

    /// How this round's results are delivered once every call in it has terminalized.
    ///
    /// Resolved once, where the round begins, because the answer cannot change while the round runs
    /// and re-deriving it per call site is a second, silent copy of the name-to-origin decision.
    ///
    /// An External MCP call always returns to the model.  A per-tool delivery policy is a Device
    /// MCP concept, and speaking a server-side answer directly would both bypass the model that
    /// asked for it and let a tool's own policy mean something it was never documented to mean.
    fn round_result_delivery(&self, calls: &[ToolCall]) -> McpResultDelivery {
        calls
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
                if self.external_mcp.find(&call.name).is_some() {
                    return McpResultDelivery::LlmThenTts;
                }
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

    /// Ends the turn's tool work, whatever the reason.
    ///
    /// The completed prefix is committed because those calls did finish and a completed
    /// ToolResult must never be lost.  Everything still outstanding is dropped: the Device MCP
    /// requests lose their semantic ownership, and the in-flight External Tool Call loses the slot
    /// that would have accepted its result — so a response that arrives afterwards is late, and a
    /// late response becomes nothing at all.
    pub(super) fn cancel_tool_turn(&mut self) {
        self.cancel_pending_mcp_turn();
        if let Some(batch) = self.tool_batch.take() {
            self.commit_tool_exchange(&batch);
        }
        self.llm_round = None;
    }
}

/// Installs a candidate's already-loaded runtime handles.
///
/// No Provider is constructed here: every handle was resolved against the process-wide Loaded
/// Runtime catalog at admission, so this only re-points the session at runtimes that already
/// exist.
///
/// A worker lease belongs to the runtime that granted it, so an open VAD or ASR lease is closed on
/// its own runtime before any pointer moves and the capture lifecycle re-arms afterwards.  A
/// runtime the candidate does not actually change is left completely untouched: re-registering an
/// unchanged runtime would strand the cleanup acknowledgement a cancelled stream still owes.
///
/// The fallible part is built first, so a rejected install leaves the active profile and its
/// runtimes exactly as they were.
fn install_candidate_runtimes(
    actor: &mut SessionActor,
    runtimes: &ResolvedAgentRuntimes,
) -> Result<(), crate::audio::AudioError> {
    let speech_output = SpeechOutput::with_worker(
        runtimes.tts.provider(),
        std::sync::Arc::clone(&runtimes.tts),
        actor.speech_output_config.clone(),
    )?;

    if !std::sync::Arc::ptr_eq(&actor.vad_runtime, &runtimes.vad) {
        // Close on the runtime that owns the lease: the candidate's VAD never granted it and would
        // refuse it as an unknown lease.
        actor.close_vad();
        actor.vad_session = None;
        actor.vad_cycle = None;
        actor.pending_vad_cycle = None;
        actor.vad_runtime.unregister_session(&actor.session_id);
        actor.vad_runtime = std::sync::Arc::clone(&runtimes.vad);
        actor.vad_events = actor.vad_runtime.register_session(&actor.session_id);
    }
    if !std::sync::Arc::ptr_eq(&actor.asr_runtime, &runtimes.asr) {
        // Cancel on the runtime that owns the lease.  No cleanup acknowledgement is outstanding at
        // a turn boundary, so nothing the departing runtime still owes this session can be lost.
        actor.cancel_asr();
        actor.asr_cleanup_pending.clear();
        actor.asr_runtime.unregister_session(&actor.session_id);
        actor.asr_runtime = std::sync::Arc::clone(&runtimes.asr);
        actor.asr_events = actor.asr_runtime.register_session(&actor.session_id);
    }
    if !std::sync::Arc::ptr_eq(&actor.llm_runtime, &runtimes.llm) {
        actor.llm_runtime.unregister_session(&actor.session_id);
        actor.llm_runtime = std::sync::Arc::clone(&runtimes.llm);
        actor.llm_events = actor
            .llm_runtime
            .register_session(&actor.session_id, LLM_EVENT_CAPACITY);
    }
    if !std::sync::Arc::ptr_eq(&actor.tts_runtime, &runtimes.tts) {
        actor.speech_output.release();
        actor.tts_runtime = std::sync::Arc::clone(&runtimes.tts);
        actor.speech_output = speech_output;
    }
    // Segmentation timing belongs to the VAD instance, so it is rebound with the VAD runtime.
    actor.vad_segmenter = VadSegmenter::new(runtimes.vad_segmenter);
    actor.pre_roll_samples = runtimes.vad_pre_roll_samples;
    actor.auto_retention = AutoPcmRetention::new(auto_retention_capacity(
        runtimes.vad.runtime_config().command_capacity,
        runtimes.vad_segmenter.min_speech_samples,
        runtimes.vad_pre_roll_samples,
    ));
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        audio::VadSegmenterConfig,
        config::AppConfig,
        config::EffectiveProviderBindings,
        database::{AdmittedAssignment, AdmittedProviderBinding},
        providers::{
            RuntimeCatalog, asr::UnavailableAsr, llm::UnavailableLlm, tts::UnavailableTts,
            vad::UnavailableVad,
        },
        session::profile::resolve_effective_session_profile,
        workers::{
            AsrWorkerRuntime, LlmRuntime, TtsWorkerRuntime, VadWorkerRuntime, WorkerRuntimeConfig,
        },
    };
    use std::{collections::HashMap, sync::Arc};

    fn deployment() -> AppConfig {
        toml::from_str(
            r#"
            [server]
            bind = "127.0.0.1:0"
            public_ws_url = "ws://127.0.0.1:0/voice/v1/"

            [provider_defaults]
            vad = "vad"
            asr = "asr"
            llm = "llm"
            tts = "tts"
            "#,
        )
        .expect("the fixture configuration is valid")
    }

    fn worker() -> WorkerRuntimeConfig {
        WorkerRuntimeConfig {
            max_workers: 1,
            command_capacity: 1,
            final_timeout: std::time::Duration::from_secs(1),
            cleanup_grace: std::time::Duration::from_secs(1),
        }
    }

    /// Carries a second LLM and VAD instance so a switch is observable as a runtime change and not
    /// only as a prompt change.
    fn catalog() -> RuntimeCatalog {
        let segmenter = VadSegmenterConfig::default();
        RuntimeCatalog {
            vad: HashMap::from([
                (
                    "vad".to_owned(),
                    crate::providers::LoadedVad {
                        runtime: Arc::new(VadWorkerRuntime::new(
                            Arc::new(UnavailableVad),
                            worker(),
                        )),
                        segmenter,
                        pre_roll_samples: 4_800,
                    },
                ),
                (
                    "alternate-vad".to_owned(),
                    crate::providers::LoadedVad {
                        runtime: Arc::new(VadWorkerRuntime::new(
                            Arc::new(UnavailableVad),
                            worker(),
                        )),
                        segmenter,
                        pre_roll_samples: 9_600,
                    },
                ),
            ]),
            asr: HashMap::from([(
                "asr".to_owned(),
                Arc::new(AsrWorkerRuntime::new(Arc::new(UnavailableAsr), worker())),
            )]),
            llm: HashMap::from([
                (
                    "llm".to_owned(),
                    Arc::new(LlmRuntime::new(
                        Arc::new(UnavailableLlm),
                        1,
                        std::time::Duration::from_secs(1),
                    )),
                ),
                (
                    "alternate".to_owned(),
                    Arc::new(LlmRuntime::new(
                        Arc::new(UnavailableLlm),
                        1,
                        std::time::Duration::from_secs(1),
                    )),
                ),
            ]),
            tts: HashMap::from([(
                "tts".to_owned(),
                Arc::new(TtsWorkerRuntime::new(Arc::new(UnavailableTts), worker())),
            )]),
            vision: HashMap::new(),
        }
    }

    fn candidate(
        template_id: i64,
        key: &str,
        prompt: &str,
        llm: &str,
        vad: &str,
    ) -> AdmittedAssignment {
        AdmittedAssignment {
            template_id,
            template_key: key.to_owned(),
            template_name: key.to_owned(),
            language: "vi-VN".to_owned(),
            prompt: prompt.to_owned(),
            template_enabled: true,
            template_revision: 2,
            is_default: template_id == 1,
            assignment_enabled: true,
            bindings: [("vad", vad), ("asr", "asr"), ("llm", llm), ("tts", "tts")]
                .into_iter()
                .map(|(provider_type, provider_key)| AdmittedProviderBinding {
                    provider_type: provider_type.to_owned(),
                    provider_key: provider_key.to_owned(),
                    provider_enabled: true,
                })
                .collect(),
        }
    }

    /// A session admitted to the default Template and one switchable candidate.
    fn admitted_actor(catalog: &RuntimeCatalog) -> SessionActor {
        let profile = resolve_effective_session_profile(
            1,
            2,
            "agent",
            &[
                candidate(1, "primary", "primary prompt", "llm", "vad"),
                candidate(7, "usable", "usable prompt", "alternate", "alternate-vad"),
            ],
            &deployment(),
            catalog,
        )
        .expect("the default template resolves against the loaded catalog");
        let admitted = profile.into_admitted_profile();
        let (active, switch_catalog) = (admitted.active, admitted.switch_catalog);
        let bound = catalog
            .resolve(&EffectiveProviderBindings {
                vad: "vad".to_owned(),
                asr: "asr".to_owned(),
                llm: "llm".to_owned(),
                tts: "tts".to_owned(),
                vision: None,
            })
            .expect("the default bindings resolve");
        let (control_tx, _control_rx) = mpsc::channel(4);
        let (audio_tx, _audio_rx) = mpsc::channel(4);
        SessionActor::new_with_runtimes_and_limiter(
            "session".to_owned(),
            control_tx,
            audio_tx,
            16,
            20,
            SessionRuntimes {
                asr: bound.asr,
                vad: bound.vad,
                llm: bound.llm,
                tts: bound.tts,
                active_turn_limiter: Arc::new(ActiveTurnLimiter::new(1)),
                vad_segmenter_config: bound.vad_segmenter,
                pre_roll_samples: bound.vad_pre_roll_samples,
            },
        )
        .expect("the session audio runtime initializes")
        .with_effective_profile(active, switch_catalog, admitted.external_mcp, 4_096)
        .expect("the admitted prompt is within the bound")
    }

    #[test]
    fn a_successful_switch_installs_the_candidate_runetimes_and_advances_the_revision() {
        let catalog = catalog();
        let mut actor = admitted_actor(&catalog);
        let initial_llm = Arc::clone(&actor.llm_runtime);
        let alternate_llm = Arc::clone(&catalog.llm["alternate"]);
        assert_eq!(actor.profile_revision(), 1);

        actor.apply_template_switch("usable");

        assert_eq!(actor.profile_revision(), 2);
        assert_eq!(actor.profile.system_prompt, "usable prompt");
        assert_eq!(actor.profile.language, "vi-VN");
        assert!(
            Arc::ptr_eq(&actor.llm_runtime, &alternate_llm)
                && !Arc::ptr_eq(&actor.llm_runtime, &initial_llm),
            "a switch installs the candidate's already-loaded runtime and never reuses the old one"
        );
    }

    #[test]
    fn a_switch_to_a_candidate_this_session_never_admitted_changes_nothing() {
        let catalog = catalog();
        let mut actor = admitted_actor(&catalog);
        let initial_llm = Arc::clone(&actor.llm_runtime);

        actor.apply_template_switch("never-admitted");

        assert_eq!(actor.profile_revision(), 1);
        assert_eq!(actor.profile.system_prompt, "primary prompt");
        assert!(
            Arc::ptr_eq(&actor.llm_runtime, &initial_llm),
            "a rejected switch must not disturb the active runtimes"
        );
    }

    /// A worker lease belongs to the runtime that granted it, so a switch that rebases capture
    /// onto another already-loaded VAD must close the old lease there and re-arm, never hand it
    /// to the new runtime or leave an Auto client without capture.
    #[test]
    fn a_switch_that_rebases_the_vad_runtime_rearms_capture_instead_of_failing_closed() {
        let catalog = catalog();
        let mut actor = admitted_actor(&catalog);
        actor.start_listening(crate::protocol::ListenMode::Auto);
        assert!(
            actor.vad_session.is_some(),
            "Auto mode arms a VAD capture cycle"
        );

        actor.apply_template_switch("usable");
        assert_eq!(actor.phase, SessionPhase::Listening);

        actor.complete_recognition();

        assert_eq!(
            actor.phase,
            SessionPhase::Listening,
            "an Auto session must keep capturing after a VAD rebasing"
        );
        assert!(
            actor.vad_session.is_some(),
            "capture must be re-armed, not left holding a lease the candidate's runtime never granted"
        );
        assert!(
            Arc::ptr_eq(
                &actor.vad_runtime,
                &catalog.vad["alternate-vad"].runtime.clone()
            ),
            "the re-armed cycle must belong to the candidate's own runtime"
        );
        assert_eq!(actor.pre_roll_samples, 9_600);
    }

    /// The overwhelmingly common switch keeps VAD, ASR and TTS on the same instances. Their leases
    /// and mailboxes must then be left completely alone.
    #[test]
    fn a_switch_that_keeps_the_capture_runtimes_leaves_their_leases_open() {
        let catalog = catalog();
        let profile = resolve_effective_session_profile(
            1,
            2,
            "agent",
            &[
                candidate(1, "primary", "primary prompt", "llm", "vad"),
                candidate(7, "usable", "usable prompt", "llm", "vad"),
            ],
            &deployment(),
            &catalog,
        )
        .expect("the default template resolves against the loaded catalog");
        let admitted = profile.into_admitted_profile();
        let (active, switch_catalog) = (admitted.active, admitted.switch_catalog);
        let bound = catalog
            .resolve(&EffectiveProviderBindings {
                vad: "vad".to_owned(),
                asr: "asr".to_owned(),
                llm: "llm".to_owned(),
                tts: "tts".to_owned(),
                vision: None,
            })
            .expect("the default bindings resolve");
        let (control_tx, _control_rx) = mpsc::channel(4);
        let (audio_tx, _audio_rx) = mpsc::channel(4);
        let mut actor = SessionActor::new_with_runtimes_and_limiter(
            "session".to_owned(),
            control_tx,
            audio_tx,
            16,
            20,
            SessionRuntimes {
                asr: bound.asr,
                vad: bound.vad,
                llm: bound.llm,
                tts: bound.tts,
                active_turn_limiter: Arc::new(ActiveTurnLimiter::new(1)),
                vad_segmenter_config: bound.vad_segmenter,
                pre_roll_samples: bound.vad_pre_roll_samples,
            },
        )
        .expect("the session audio runtime initializes")
        .with_effective_profile(active, switch_catalog, admitted.external_mcp, 4_096)
        .expect("the admitted prompt is within the bound");
        actor.start_listening(crate::protocol::ListenMode::Auto);
        let lease = actor.vad_session.as_ref().map(|(lease, _)| *lease);

        actor.apply_template_switch("usable");

        assert_eq!(actor.profile.revision, 2);
        assert_eq!(
            actor.vad_session.as_ref().map(|(open, _)| *open),
            lease,
            "an unchanged capture runtime must keep the lease the session already holds"
        );
    }

    use crate::tools::external_mcp::{
        ExternalToolCatalog, ResolvedExternalMcp, ResolvedExternalTool, SessionExternalMcp,
        normalize_external_tool_segment,
    };

    /// A server handle exactly as admission produces one, from a client that is already resolved.
    ///
    /// The credential, if there is one, lives inside the client handle and nowhere the session can
    /// reach — which is the whole reason the executor is handed this and not a resolver.
    fn resolved_server(
        server_key: &str,
        original_name: &str,
        client: crate::tools::external_mcp::ExternalMcpClient,
    ) -> ResolvedExternalMcp {
        // Admission names the namespace from the normalized key, never from the raw one, so a
        // hand-built handle here is named the same way a resolved one would be.
        let namespace = format!(
            "external.{}",
            normalize_external_tool_segment(server_key).expect("a server key normalizes")
        );
        let published = ExternalToolCatalog::publish(
            &namespace,
            vec![(
                original_name.to_owned(),
                format!("{original_name} tool"),
                serde_json::json!({"type": "object"}),
            )],
        )
        .expect("a single tool publishes");
        ResolvedExternalMcp {
            server_key: server_key.to_owned(),
            namespace,
            client: Arc::new(client),
            tools: Arc::from(published.tools().to_vec()),
            call_timeout: std::time::Duration::from_secs(1),
            limiter: Arc::new(crate::tools::external_mcp::ExternalMcpCallLimiter::new(16)),
        }
    }

    /// A catalog resolved once against an allowlisted name, without any network work.
    fn admitted_external_mcp() -> SessionExternalMcp {
        struct Fixed;
        impl crate::database::secrets::SecretResolver for Fixed {
            fn resolve(
                &self,
                _: &crate::database::secrets::SecretRef,
            ) -> Result<
                crate::database::secrets::SecretValue,
                crate::database::secrets::SecretResolveError,
            > {
                Ok(crate::database::secrets::SecretValue::new("s3cr3t".into()))
            }
        }
        let reference = crate::database::secrets::SecretRef::parse("WEATHER_TOKEN".into())
            .expect("an opaque reference parses");
        let client = crate::tools::external_mcp::ExternalMcpClient::connect(
            "home-assistant",
            "https://mcp.internal.test/rpc",
            "{}",
            "bearer",
            None,
            Some(&reference),
            std::time::Duration::from_secs(1),
            std::time::Duration::from_secs(1),
            reqwest::Client::new(),
            crate::config::ExternalMcpNetworkConfig {
                allow_http_lan: false,
                allowed_hosts: vec!["mcp.internal.test".into()],
                allowed_cidrs: vec![],
            },
            &crate::config::ExternalMcpLimitsConfig::default(),
            Arc::new(crate::telemetry::TracingTelemetry),
            &Fixed,
        )
        .expect("an allowlisted destination produces a client");
        SessionExternalMcp::new(vec![resolved_server(
            "home-assistant",
            "Light/Turn-On",
            client,
        )])
    }

    fn session_with_external_mcp(external_mcp: SessionExternalMcp) -> SessionActor {
        let catalog = catalog();
        let profile = resolve_effective_session_profile(
            1,
            2,
            "agent",
            &[candidate(1, "primary", "primary prompt", "llm", "vad")],
            &deployment(),
            &catalog,
        )
        .expect("the default template resolves against the loaded catalog");
        let admitted = profile
            .with_external_mcp(external_mcp)
            .into_admitted_profile();
        let bound = catalog
            .resolve(&EffectiveProviderBindings {
                vad: "vad".to_owned(),
                asr: "asr".to_owned(),
                llm: "llm".to_owned(),
                tts: "tts".to_owned(),
                vision: None,
            })
            .expect("the default bindings resolve");
        let (control_tx, _control_rx) = mpsc::channel(4);
        let (audio_tx, _audio_rx) = mpsc::channel(4);
        SessionActor::new_with_runtimes_and_limiter(
            "session".to_owned(),
            control_tx,
            audio_tx,
            16,
            20,
            SessionRuntimes {
                asr: bound.asr,
                vad: bound.vad,
                llm: bound.llm,
                tts: bound.tts,
                active_turn_limiter: Arc::new(ActiveTurnLimiter::new(1)),
                vad_segmenter_config: bound.vad_segmenter,
                pre_roll_samples: bound.vad_pre_roll_samples,
            },
        )
        .expect("the session audio runtime initializes")
        .with_effective_profile(
            admitted.active,
            admitted.switch_catalog,
            admitted.external_mcp,
            4_096,
        )
        .expect("the admitted prompt is within the bound")
    }

    /// A round that starts after the application closed its gate executes nothing at all.
    ///
    /// The gate is what shutdown depends on, so this is the property that makes the drain
    /// meaningful: a session that never observed the shutdown signal still cannot put a call on the
    /// network once the application has stopped accepting work.
    #[test]
    fn a_closed_gate_starts_no_tool_round_at_all() {
        let gate = AdmissionGate::open();
        let mut actor =
            session_with_external_mcp(admitted_external_mcp()).with_admission_gate(Arc::clone(&gate));
        actor
            .begin_active_turn()
            .expect("an active turn is admitted");
        actor
            .commit_user_text("turn on the light".to_owned())
            .expect("the accepted user text is committed to the turn");

        gate.close();
        actor.start_tool_batch(vec![ToolCall {
            id: "call-1".to_owned(),
            name: "external.home_assistant.light_turn_on".to_owned(),
            arguments: serde_json::json!({}),
        }]);

        assert!(
            actor.tool_batch.is_none(),
            "no round is opened, so no call can be dispatched from it"
        );
        assert!(
            tool_round(&actor).is_empty(),
            "nothing was called, so there is no ToolResult standing in for a call"
        );
    }

    /// A round that was already running when the gate closed cannot use the next slot in its own
    /// budget to start another call.
    ///
    /// This is the half the round-start check cannot cover: the call already on the network was
    /// permitted, and it still keeps its own paired result.  Only what would have been sent after
    /// the gate closed is refused.
    #[tokio::test]
    async fn a_closed_gate_stops_a_round_that_was_already_running() {
        let server = GatedServer::start().await;
        let telemetry = Arc::new(crate::telemetry::RecordingTelemetry::default());
        let gate = AdmissionGate::open();
        let mut actor =
            session_with_external_mcp(gated_snapshot(&server, Arc::clone(&telemetry)))
                .with_admission_gate(Arc::clone(&gate));
        actor
            .begin_active_turn()
            .expect("an active turn is admitted");
        actor
            .commit_user_text("what is the forecast".to_owned())
            .expect("the accepted user text is committed to the turn");

        let call = |id: &str| ToolCall {
            id: id.to_owned(),
            name: "external.weather.forecast".to_owned(),
            arguments: serde_json::json!({}),
        };
        actor.start_tool_batch(vec![call("call-1"), call("call-2")]);
        server.wait_until_held().await;

        // The gate closes while the first call is still on the network.
        gate.close();
        server.release();

        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                actor.drain_external_call_completions();
                if !tool_round(&actor).is_empty() {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the call that was already in flight reports its outcome");
        actor.drain_external_call_completions();

        assert_eq!(
            tool_round(&actor),
            vec!["call-1".to_owned()],
            "the call that had already been sent keeps its paired result, and the second is not \
             paired because it never ran"
        );
        assert!(
            actor.tool_batch.is_none(),
            "the refused call terminalized the round rather than leaving it half-executed"
        );
        assert_eq!(
            counted(
                &telemetry,
                crate::telemetry::EXTERNAL_MCP_TOOL_CALLS_TOTAL
            ),
            1,
            "exactly one request reached the network"
        );
    }

    /// A Voice Session runs with exactly the External MCP snapshot admission resolved: the
    /// namespaced names it may call, and the original wire name each one is called with.
    #[test]
    fn a_session_installs_the_admitted_external_mcp_catalog() {
        let actor = session_with_external_mcp(admitted_external_mcp());
        let installed = actor.session_external_mcp();
        assert_eq!(installed.tool_count(), 1);
        let (server, tool) = installed
            .find("external.home_assistant.light_turn_on")
            .expect("the admitted tool is routable");
        assert_eq!(server.server_key, "home-assistant");
        assert_eq!(server.call_timeout, std::time::Duration::from_secs(1));
        assert_eq!(tool.original_name, "Light/Turn-On");
        // A Device MCP name, or a name from a server this session never admitted, resolves to
        // nothing: routing goes through the origin, not through the visible name.
        assert!(installed.find("self.light_turn_on").is_none());
        assert!(installed.find("external.other.light_turn_on").is_none());
        assert!(installed.find("external.home_assistant.dim").is_none());
    }

    /// Nothing a session holds renders a credential: the handle owns it and prints none of it.
    #[test]
    fn a_sessions_external_mcp_snapshot_renders_without_a_credential() {
        let installed = admitted_external_mcp();
        let rendered = format!("{:?}", installed);
        assert!(rendered.contains("external.home_assistant.light_turn_on"));
        assert!(!rendered.contains("s3cr3t"), "{rendered}");
        assert!(!rendered.contains("WEATHER_TOKEN"), "{rendered}");
    }

    /// The published names are the guide's fixed namespace, and the tool behind one keeps the
    /// original wire name it is actually called with.
    #[test]
    fn external_tool_names_are_namespaced_and_keep_their_wire_name() {
        assert_eq!(
            normalize_external_tool_segment("Home-Assistant").as_deref(),
            Some("home_assistant")
        );
        let published = ExternalToolCatalog::publish(
            "external.home_assistant",
            vec![(
                "Light/Turn-On".to_owned(),
                "turns a light on".to_owned(),
                serde_json::json!({"type": "object"}),
            )],
        )
        .expect("a single tool publishes");
        let tool: &ResolvedExternalTool = &published.tools()[0];
        assert_eq!(tool.llm_name, "external.home_assistant.light_turn_on");
        assert_eq!(tool.original_name, "Light/Turn-On");
    }

    #[test]
    fn a_session_without_a_catalog_is_never_offered_the_switch_tool() {
        let catalog = catalog();
        let bound = catalog
            .resolve(&EffectiveProviderBindings {
                vad: "vad".to_owned(),
                asr: "asr".to_owned(),
                llm: "llm".to_owned(),
                tts: "tts".to_owned(),
                vision: None,
            })
            .expect("the default bindings resolve");
        let (control_tx, _control_rx) = mpsc::channel(4);
        let (audio_tx, _audio_rx) = mpsc::channel(4);
        let actor = SessionActor::new_with_runtimes_and_limiter(
            "session".to_owned(),
            control_tx,
            audio_tx,
            16,
            20,
            SessionRuntimes {
                asr: bound.asr,
                vad: bound.vad,
                llm: bound.llm,
                tts: bound.tts,
                active_turn_limiter: Arc::new(ActiveTurnLimiter::new(1)),
                vad_segmenter_config: bound.vad_segmenter,
                pre_roll_samples: bound.vad_pre_roll_samples,
            },
        )
        .expect("the session audio runtime initializes");

        assert!(
            actor
                .available_llm_tools()
                .iter()
                .all(|tool| tool.name != SWITCH_TEMPLATE_TOOL_NAME),
            "a session with no admission catalog cannot offer a switch"
        );
    }

    // ---------------------------------------------------------------------------
    // The late-response window
    // ---------------------------------------------------------------------------

    /// A local External MCP server whose `tools/call` answer is held until the test opens it.
    ///
    /// A gate rather than a sleep is what makes the window below reachable on purpose: the test
    /// chooses the instant the response becomes available, instead of hoping a timer lines up.
    struct GatedServer {
        url: String,
        held: Arc<tokio::sync::Notify>,
        opener: watch::Sender<bool>,
        #[allow(dead_code)]
        task: tokio::task::JoinHandle<()>,
    }

    impl GatedServer {
        async fn start() -> Self {
            let held = Arc::new(tokio::sync::Notify::new());
            let (opener, open) = watch::channel(false);
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("a loopback listener binds");
            let address = listener.local_addr().expect("the listener has an address");
            let route_held = Arc::clone(&held);
            let route_open = open.clone();
            let router = axum::Router::new().route(
                "/mcp",
                axum::routing::post(move |body: axum::body::Bytes| {
                    let held = Arc::clone(&route_held);
                    let mut open = route_open.clone();
                    async move {
                        let request: serde_json::Value =
                            serde_json::from_slice(&body).unwrap_or_default();
                        let id = request.get("id").cloned();
                        let result = match request.get("method").and_then(serde_json::Value::as_str)
                        {
                            Some("initialize") => serde_json::json!({
                                "protocolVersion": "2024-11-05",
                                "capabilities": {"tools": {}},
                                "serverInfo": {"name": "gated", "version": "1"}
                            }),
                            Some("tools/list") => serde_json::json!({"tools": [{
                                "name": "Forecast",
                                "description": "forecast",
                                "inputSchema": {"type": "object", "properties": {},
                                                "additionalProperties": false}
                            }]}),
                            Some("tools/call") => {
                                held.notify_one();
                                while !*open.borrow_and_update() {
                                    open.changed().await.ok();
                                }
                                serde_json::json!({
                                    "content": [{"type": "text", "text": "late"}],
                                    "isError": false
                                })
                            }
                            _ => serde_json::json!({}),
                        };
                        let mut document = serde_json::json!({"jsonrpc": "2.0", "result": result});
                        if let Some(id) = id {
                            document["id"] = id;
                        }
                        use axum::response::IntoResponse;
                        (
                            axum::http::StatusCode::OK,
                            [(
                                axum::http::header::CONTENT_TYPE,
                                "application/json".to_owned(),
                            )],
                            document.to_string(),
                        )
                            .into_response()
                    }
                }),
            );
            let task = tokio::spawn(async move {
                axum::serve(listener, router).await.ok();
            });
            Self {
                url: format!("http://{address}/mcp"),
                held,
                opener,
                task,
            }
        }

        async fn wait_until_held(&self) {
            self.held.notified().await;
        }

        fn release(&self) {
            let _ = self.opener.send(true);
        }
    }

    /// A snapshot whose calls report into a sink the test can read.
    fn gated_snapshot(
        server: &GatedServer,
        telemetry: Arc<crate::telemetry::RecordingTelemetry>,
    ) -> SessionExternalMcp {
        let client = crate::tools::external_mcp::ExternalMcpClient::connect(
            "weather",
            &server.url,
            "{}",
            "none",
            None,
            None,
            std::time::Duration::from_secs(1),
            std::time::Duration::from_secs(2),
            reqwest::Client::new(),
            crate::config::ExternalMcpNetworkConfig {
                allow_http_lan: true,
                allowed_hosts: vec![],
                allowed_cidrs: vec!["127.0.0.0/8".into()],
            },
            &crate::config::ExternalMcpLimitsConfig::default(),
            telemetry,
            &crate::database::secrets::EnvSecretResolver,
        )
        .expect("an allowlisted loopback destination produces a client");
        SessionExternalMcp::new(vec![resolved_server("weather", "Forecast", client)])
    }

    fn counted(telemetry: &Arc<crate::telemetry::RecordingTelemetry>, metric: &str) -> usize {
        telemetry
            .recorded()
            .iter()
            .filter(|entry| entry.metric == metric)
            .count()
    }

    /// Waits for one observation, which is the exit condition rather than a delay chosen in
    /// advance of knowing how long the observation takes.
    async fn await_count(telemetry: &Arc<crate::telemetry::RecordingTelemetry>, metric: &str) {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if counted(telemetry, metric) > 0 {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("{metric} is observed within its own budget"));
    }

    /// The completed tool pairs the turn is holding, in order.
    fn tool_round(actor: &SessionActor) -> Vec<String> {
        actor
            .llm_messages
            .iter()
            .filter_map(|message| match message {
                ChatMessage::ToolResult { tool_call_id, .. } => Some(tool_call_id.clone()),
                _ => None,
            })
            .collect()
    }

    /// A response that lands after the round that started it is gone becomes nothing at all.
    ///
    /// This is the window a socket test cannot order: in production the call's task posts its
    /// completion into the session's bounded mailbox, and an abort arriving before the next drain
    /// takes the round away first.  The test reproduces that end state exactly — the round is gone
    /// before the response is even allowed to exist — so the executor has only one thing it could
    /// possibly do with what arrives, and the assertion is on what it does instead.
    #[tokio::test]
    async fn a_response_that_lands_after_its_round_is_discarded_and_acts_on_nothing() {
        let server = GatedServer::start().await;
        let telemetry = Arc::new(crate::telemetry::RecordingTelemetry::default());
        let mut actor = session_with_external_mcp(gated_snapshot(&server, Arc::clone(&telemetry)));
        actor
            .begin_active_turn()
            .expect("an active turn is admitted");
        actor
            .commit_user_text("what is the forecast".to_owned())
            .expect("the accepted user text is committed to the turn");

        actor.start_tool_batch(vec![ToolCall {
            id: "call-1".to_owned(),
            name: "external.weather.forecast".to_owned(),
            arguments: serde_json::json!({}),
        }]);
        assert!(
            actor
                .tool_batch
                .as_ref()
                .is_some_and(|batch| batch.in_flight.is_some()),
            "the round waits for exactly one call, and never more than one"
        );

        server.wait_until_held().await;
        // The turn ends while the call is still in flight and its answer still held, so the
        // response cannot have been applied by anything: the round it belonged to no longer exists.
        actor.cancel_tool_turn();
        assert!(actor.advance_generation());
        server.release();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                actor.drain_external_call_completions();
                if counted(
                    &telemetry,
                    crate::telemetry::EXTERNAL_TOOL_LATE_RESPONSE_DISCARDED_TOTAL,
                ) > 0
                {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("the late response reaches the session's mailbox");

        assert_eq!(
            counted(
                &telemetry,
                crate::telemetry::EXTERNAL_TOOL_LATE_RESPONSE_DISCARDED_TOTAL
            ),
            1,
            "the late response is counted once, against the server that produced it"
        );
        assert!(
            actor.tool_batch.is_none(),
            "a discarded response starts no round, so nothing is left to finish"
        );
        assert!(
            tool_round(&actor).is_empty(),
            "a discarded response produces no ToolResult in the prompt base snapshot"
        );
        let turn_id = actor.current_turn_id().expect("the turn is still open");
        let history = actor
            .dialogue_history
            .messages_for_prompt(turn_id)
            .expect("the turn's history renders");
        assert!(
            history
                .iter()
                .all(|message| !matches!(message, ChatMessage::ToolResult { .. })),
            "a discarded response produces no completed round in history either"
        );
    }

    /// An interrupted turn drops the call it had in flight rather than waiting for it.
    ///
    /// The turn's own cancellation is what the executor observes, so this goes through the same
    /// `cancel_llm` seam a barge-in, a client abort and a shutdown all take.
    #[tokio::test]
    async fn an_interrupted_turn_drops_its_in_flight_call_without_producing_a_result() {
        let server = GatedServer::start().await;
        let telemetry = Arc::new(crate::telemetry::RecordingTelemetry::default());
        let mut actor = session_with_external_mcp(gated_snapshot(&server, Arc::clone(&telemetry)));
        actor
            .begin_active_turn()
            .expect("an active turn is admitted");

        actor.start_tool_batch(vec![ToolCall {
            id: "call-1".to_owned(),
            name: "external.weather.forecast".to_owned(),
            arguments: serde_json::json!({}),
        }]);
        server.wait_until_held().await;

        // Cancellation first, then the round: the order every interruption path uses.
        actor.cancel_llm();
        actor.cancel_tool_turn();
        server.release();
        await_count(
            &telemetry,
            crate::telemetry::EXTERNAL_TOOL_CALL_CANCELLED_TOTAL,
        )
        .await;
        actor.drain_external_call_completions();

        assert_eq!(
            counted(
                &telemetry,
                crate::telemetry::EXTERNAL_TOOL_CALL_CANCELLED_TOTAL
            ),
            1,
            "the in-flight call is reported as dropped exactly once"
        );
        assert_eq!(
            counted(
                &telemetry,
                crate::telemetry::EXTERNAL_TOOL_LATE_RESPONSE_DISCARDED_TOTAL
            ),
            0,
            "a call dropped by cancellation is never answered, so nothing is ever discarded"
        );
        assert!(
            tool_round(&actor).is_empty(),
            "a dropped call produces no ToolResult, so no continuation can be built on one"
        );
    }
}
