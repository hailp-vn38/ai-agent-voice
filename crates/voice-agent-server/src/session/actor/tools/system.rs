//! System MCP shares the sequential executor's bounded completion mailbox.
use super::*;
use crate::tools::system_mcp::SessionSystemMcp;

impl SessionActor {
    pub(crate) fn with_system_mcp(mut self, system: Option<SessionSystemMcp>) -> Self {
        self.system_mcp = system;
        self
    }

    pub(in super::super) fn dispatch_system_tool(
        &mut self,
        call: ToolCall,
        mut system: SessionSystemMcp,
        budget: std::time::Duration,
    ) {
        let Some(turn) = &self.turn else {
            self.complete_tool_call(call, Err("system_tool_unavailable"));
            return;
        };
        let (turn_id, generation) = (turn.turn_id, turn.generation);
        system.context.cancellation = turn.cancellation.clone();
        system.context.deadline = tokio::time::Instant::now() + budget;
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            self.complete_tool_call(call, Err("system_tool_unavailable"));
            return;
        };
        let completions = self.async_calls_tx.clone();
        let dispatched = call.clone();
        runtime.spawn(async move {
            let outcome = system
                .core
                .call_tool(&system.context, &dispatched.name, dispatched.arguments)
                .await;
            if system.context.cancellation.is_cancelled() {
                return;
            }
            let _ = completions.try_send(AsyncCallCompletion {
                turn_id,
                generation,
                call_id: dispatched.id,
                server_key: String::new(),
                outcome: AsyncToolOutcome::System(outcome),
            });
        });
        if let Some(batch) = &mut self.tool_batch {
            batch.in_flight = Some(InFlightAsyncCall {
                call,
                turn_id,
                generation,
            });
        }
    }
}
