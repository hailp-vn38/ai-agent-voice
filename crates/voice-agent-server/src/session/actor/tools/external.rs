//! External MCP's asynchronous call adapter and its bounded completion mailbox.

use super::*;
use std::sync::Arc;

use crate::{
    providers::llm::ToolCall,
    tools::{
        external_mcp::{ExternalMcpError, ResolvedExternalMcp, ResolvedExternalTool},
        round::external_tool_result,
    },
};

impl SessionActor {
    /// Starts the single outbound attempt this External Tool Call is allowed to make.
    ///
    /// There is no retry: a side effect may already have happened remotely before a timeout or a
    /// dropped connection, so a second attempt is not a recovery strategy.
    pub(in super::super) fn dispatch_external_tool(
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
        let Some(guard) = self.external_mcp.guard.clone() else {
            self.complete_external_tool_call(call, Err(&ExternalMcpError::ToolUnavailable));
            return;
        };
        runtime.spawn(async move {
            let outcome = tokio::select! {
                biased;
                () = cancellation.cancelled() => {
                    client.telemetry().call_cancelled(&server_key);
                    return;
                }
                outcome = client.call_tool_guarded(&limiter, &tool, &arguments, budget, &guard) => outcome,
            };
            let completion = ExternalCallCompletion {
                turn_id,
                generation,
                call_id,
                server_key: server_key.clone(),
                outcome,
            };
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

    /// Drops completions which no longer match the one External call in flight.
    pub(in super::super) fn drain_external_call_completions(&mut self) {
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

    fn external_telemetry(&self, server_key: &str) -> Option<Arc<dyn crate::telemetry::Telemetry>> {
        self.external_mcp
            .servers()
            .iter()
            .find(|server| server.server_key == server_key)
            .map(|server| Arc::clone(server.client.telemetry()))
    }
}
