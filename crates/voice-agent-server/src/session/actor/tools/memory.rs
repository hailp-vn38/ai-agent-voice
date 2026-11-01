//! Memory builtins. Fact text lives in the arguments and results, so neither is ever logged
//! (ADR 0024): the lines here carry the tool, the call, the session and a result code.

use super::*;

use crate::{
    services::memory::RecallQuery,
    session::SessionMemory,
    tools::builtin::{MAX_MEMORY_RECALL_LIMIT, MEMORY_RECALL_TOOL_NAME, parse_memory_recall_args},
};

const RECALL_BODY_MAX_CHARS: usize = 500;

impl SessionActor {
    pub(in super::super) fn execute_memory_tool(&mut self, call: ToolCall) {
        let Some(memory) = self.session_memory.clone() else {
            self.reject_memory_tool(call, "memory_disabled");
            return;
        };
        let Some(turn) = self.turn.as_ref() else {
            self.reject_memory_tool(call, "no_active_turn");
            return;
        };
        let (turn_id, generation) = (turn.turn_id, turn.generation);
        let cancellation = turn.cancellation.clone();
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            self.reject_memory_tool(call, "memory_unavailable");
            return;
        };
        tracing::info!(
            event = "voice_memory_tool_called",
            session_id = %self.session_id,
            turn_id = turn_id.get(),
            tool = %call.name,
            call_id = %call.id,
        );
        let completions = self.async_calls_tx.clone();
        let session_id = self.session_id.clone();
        let dispatched = call.clone();
        runtime.spawn(async move {
            let outcome = tokio::select! {
                biased;
                () = cancellation.cancelled() => return,
                outcome = run_memory_tool(&memory, &dispatched) => outcome,
            };
            tracing::info!(
                event = "voice_memory_tool_completed",
                session_id = %session_id,
                tool = %dispatched.name,
                call_id = %dispatched.id,
                code = outcome.as_ref().err().copied().unwrap_or("ok"),
            );
            let _ = completions.try_send(AsyncCallCompletion {
                turn_id,
                generation,
                call_id: dispatched.id,
                server_key: String::new(),
                outcome: AsyncToolOutcome::Server(outcome),
            });
        });
        if let Some(batch) = self.tool_batch.as_mut() {
            batch.in_flight = Some(InFlightAsyncCall {
                call,
                turn_id,
                generation,
            });
        }
    }

    fn reject_memory_tool(&mut self, call: ToolCall, code: &'static str) {
        tracing::warn!(
            event = "voice_memory_tool_rejected",
            session_id = %self.session_id,
            tool = %call.name,
            call_id = %call.id,
            code,
        );
        self.complete_tool_call(call, Err(code));
    }
}

/// One arm per Memory builtin. The error is the tool result's code.
async fn run_memory_tool(
    memory: &SessionMemory,
    call: &ToolCall,
) -> Result<serde_json::Value, &'static str> {
    match call.name.as_str() {
        MEMORY_RECALL_TOOL_NAME => recall(memory, &call.arguments).await,
        _ => Err("memory_tool_unknown"),
    }
}

async fn recall(
    memory: &SessionMemory,
    arguments: &serde_json::Value,
) -> Result<serde_json::Value, &'static str> {
    let args = parse_memory_recall_args(arguments).map_err(|_| "invalid_arguments")?;
    let limit = args
        .limit
        .unwrap_or(MAX_MEMORY_RECALL_LIMIT)
        .min(MAX_MEMORY_RECALL_LIMIT);
    let found = memory
        .core
        .recall(
            &memory.scope,
            RecallQuery {
                text: args.query,
                limit,
            },
        )
        .await
        .map_err(|error| error.code())?;
    let items: Vec<serde_json::Value> = found
        .items
        .iter()
        .map(|fact| {
            let body: String = fact.body.chars().take(RECALL_BODY_MAX_CHARS).collect();
            serde_json::json!({
                "id": fact.id,
                "revision": fact.revision,
                "title": fact.title,
                "hook": fact.hook,
                "body": body,
                "kind": fact.kind,
                "updated_at": fact.updated_at.to_rfc3339(),
            })
        })
        .collect();
    Ok(serde_json::json!({
        "untrusted_data": true,
        "items": items,
        "hidden": found.hidden,
    }))
}
