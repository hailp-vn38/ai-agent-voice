//! Memory builtins. Fact text lives in the arguments and results, so neither is ever logged
//! (ADR 0024): the lines here carry the tool, the call, the session and a result code.

use super::*;

use crate::{
    services::memory::{
        ExplicitIntent, FactId, MemoryActor, MemoryChange, MemoryContext, RecallQuery,
        classify_intent,
    },
    session::SessionMemory,
    tools::builtin::{
        BuiltinToolError, MAX_MEMORY_RECALL_LIMIT, MEMORY_CORRECT_TOOL_NAME,
        MEMORY_FORGET_TOOL_NAME, MEMORY_RECALL_TOOL_NAME, MEMORY_REMEMBER_TOOL_NAME,
        is_memory_write_tool_name, memory_tool_definitions, parse_memory_correct_args,
        parse_memory_forget_args, parse_memory_recall_args, parse_memory_remember_args,
    },
};
use unicode_normalization::UnicodeNormalization;

const RECALL_BODY_MAX_CHARS: usize = 500;
/// What a successful Forget cannot have done, for the model to say out loud.
const FORGET_LIMITS: [&str; 3] = [
    "still_in_this_turn",
    "still_in_session_history",
    "copies_outside_memory_not_deleted",
];
const CLAIM_PHRASES: [&str; 3] = ["đã nhớ", "đã quên", "đã lưu"];

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
        self.memory_call_turn = Some(turn_id);
        tracing::info!(
            subsystem = "voice.llm",
            event = "voice_memory_tool_called",
            session_id = %self.session_id,
            turn_id = turn_id.get(),
            tool = %call.name,
            call_id = %call.id,
            "Memory tool called"
        );
        let intent = self.turn_intent();
        let completions = self.async_calls_tx.clone();
        let session_id = self.session_id.clone();
        let dispatched = call.clone();
        runtime.spawn(async move {
            let started = std::time::Instant::now();
            // A barge-in drops this future, not the write: `apply` runs in a task of its own.
            let outcome = tokio::select! {
                biased;
                () = cancellation.cancelled() => {
                    tracing::info!(
                        subsystem = "voice.llm",
                        event = "voice_memory_tool_cancelled",
                        session_id = %session_id,
                        turn_id = turn_id.get(),
                        tool = %dispatched.name,
                        call_id = %dispatched.id,
                        "Memory tool cancelled with its turn"
                    );
                    return;
                }
                outcome = run_memory_tool(&memory, intent, &dispatched) => outcome,
            };
            let (recalled_items, recalled_hidden) = recall_counts(&outcome);
            tracing::info!(
                subsystem = "voice.llm",
                event = "voice_memory_tool_completed",
                session_id = %session_id,
                turn_id = turn_id.get(),
                tool = %dispatched.name,
                call_id = %dispatched.id,
                code = outcome.as_ref().err().copied().unwrap_or("ok"),
                recalled_items,
                recalled_hidden,
                duration_ms = started.elapsed().as_millis() as u64,
                "Memory tool completed"
            );
            let delivered = completions.try_send(AsyncCallCompletion {
                turn_id,
                generation,
                call_id: dispatched.id.clone(),
                server_key: String::new(),
                outcome: AsyncToolOutcome::Server(outcome),
            });
            if delivered.is_err() {
                tracing::warn!(
                    subsystem = "voice.llm",
                    event = "voice_memory_tool_result_undelivered",
                    session_id = %session_id,
                    turn_id = turn_id.get(),
                    tool = %dispatched.name,
                    call_id = %dispatched.id,
                    "Memory tool result did not reach the session"
                );
            }
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
            subsystem = "voice.llm",
            event = "voice_memory_tool_rejected",
            session_id = %self.session_id,
            tool = %call.name,
            call_id = %call.id,
            code,
            "Memory tool rejected"
        );
        self.complete_tool_call(call, Err(code));
    }

    /// Read from the last `ChatMessage::User`, the text the server committed for this turn. Tool
    /// results have their own variant, so nothing a tool returns can open the gate.
    fn turn_intent(&self) -> ExplicitIntent {
        self.llm_messages
            .iter()
            .rev()
            .find_map(|message| match message {
                ChatMessage::User { content } => Some(classify_intent(content)),
                _ => None,
            })
            .unwrap_or_default()
    }

    /// Only a write's success is a Memory Receipt; a recall proves nothing was saved.
    pub(in super::super) fn note_memory_receipt(&mut self, tool: &str, turn_id: TurnId) {
        if is_memory_write_tool_name(tool) {
            self.memory_receipt_turn = Some(turn_id);
        }
    }

    /// What this turn's model request holds of Memory: row counts only, never the rows.
    pub(in super::super) fn observe_memory_context(
        &self,
        turn_id: TurnId,
        context: &MemoryContext,
        system: &str,
    ) {
        let Some(memory) = &self.session_memory else {
            return;
        };
        tracing::info!(
            subsystem = "voice.llm",
            event = "voice_memory_context_composed",
            session_id = %self.session_id,
            turn_id = turn_id.get(),
            device_db_id = memory.device_db_id,
            mode = ?memory.mode,
            summary_rows = context.rows.len(),
            summary_hidden = context.hidden,
            in_prompt = system.contains("<memory_info>"),
            tools_offered = memory_tool_definitions(memory.mode).len(),
            llm_supports_tools = self.llm_runtime.supports_tools(),
            "Memory context composed for the turn"
        );
    }

    /// Whether the model used Memory for its final answer, and promises it did not keep. It adds
    /// nothing to what the user hears.
    pub(in super::super) fn observe_memory_claim(&self) {
        let Some(turn_id) = self.current_turn_id() else {
            return;
        };
        if self.session_memory.is_none() {
            return;
        }
        let has_receipt = self.memory_receipt_turn == Some(turn_id);
        tracing::info!(
            subsystem = "voice.llm",
            event = "voice_memory_turn_answered",
            session_id = %self.session_id,
            turn_id = turn_id.get(),
            memory_tool_called = self.memory_call_turn == Some(turn_id),
            has_receipt,
            "Turn answered with Memory available"
        );
        if !has_receipt && claims_a_memory_write(&self.generated_response) {
            tracing::warn!(
                subsystem = "voice.llm",
                event = "memory_claim_without_receipt",
                session_id = %self.session_id,
                turn_id = turn_id.get(),
                "Answer claims a Memory write without a receipt"
            );
        }
    }
}

/// A recall's item counts, for its log line. A write or a failure has none.
fn recall_counts(outcome: &Result<serde_json::Value, &'static str>) -> (Option<u64>, Option<u64>) {
    let Ok(result) = outcome else {
        return (None, None);
    };
    let items = result
        .get("items")
        .and_then(serde_json::Value::as_array)
        .map(|items| items.len() as u64);
    (
        items,
        result.get("hidden").and_then(serde_json::Value::as_u64),
    )
}

fn claims_a_memory_write(answer: &str) -> bool {
    let answer: String = answer.nfc().flat_map(char::to_lowercase).collect();
    CLAIM_PHRASES.iter().any(|phrase| answer.contains(phrase))
}

/// One arm per Memory builtin. The error is the tool result's code.
async fn run_memory_tool(
    memory: &SessionMemory,
    intent: ExplicitIntent,
    call: &ToolCall,
) -> Result<serde_json::Value, &'static str> {
    let actor = MemoryActor::VoiceUser {
        device_db_id: memory.device_db_id,
        mode: memory.mode,
        intent,
    };
    match call.name.as_str() {
        MEMORY_RECALL_TOOL_NAME => recall(memory, &call.arguments).await,
        MEMORY_REMEMBER_TOOL_NAME => remember(memory, actor, &call.arguments).await,
        MEMORY_CORRECT_TOOL_NAME => correct(memory, actor, &call.arguments).await,
        MEMORY_FORGET_TOOL_NAME => forget(memory, actor, &call.arguments).await,
        _ => Err("memory_tool_unknown"),
    }
}

fn invalid_arguments(_: BuiltinToolError) -> &'static str {
    "invalid_arguments"
}

/// An id that is not a fact id, such as a Wiki Page's, names no fact.
fn fact_id(id: &str) -> Result<FactId, &'static str> {
    FactId::parse(id).ok_or("memory_not_found")
}

/// Permissions, the intent gate and every limit are `apply`'s to enforce.
async fn write(
    memory: &SessionMemory,
    actor: MemoryActor,
    change: MemoryChange,
) -> Result<serde_json::Value, &'static str> {
    let receipt = memory
        .core
        .apply(&memory.scope, actor, change)
        .await
        .map_err(|error| error.code())?;
    Ok(serde_json::json!({"ok": true, "receipt": receipt}))
}

async fn remember(
    memory: &SessionMemory,
    actor: MemoryActor,
    arguments: &serde_json::Value,
) -> Result<serde_json::Value, &'static str> {
    let args = parse_memory_remember_args(arguments).map_err(invalid_arguments)?;
    let change = MemoryChange::Remember {
        title: args.title,
        hook: args.hook,
        body: args.body,
        kind: args.kind,
        source_ref: None,
    };
    write(memory, actor, change).await
}

async fn correct(
    memory: &SessionMemory,
    actor: MemoryActor,
    arguments: &serde_json::Value,
) -> Result<serde_json::Value, &'static str> {
    let args = parse_memory_correct_args(arguments).map_err(invalid_arguments)?;
    let change = MemoryChange::Correct {
        id: fact_id(&args.id)?,
        expected_revision: args.revision,
        title: args.title,
        hook: args.hook,
        body: args.body,
    };
    write(memory, actor, change).await
}

async fn forget(
    memory: &SessionMemory,
    actor: MemoryActor,
    arguments: &serde_json::Value,
) -> Result<serde_json::Value, &'static str> {
    let args = parse_memory_forget_args(arguments).map_err(invalid_arguments)?;
    let change = MemoryChange::Forget {
        id: fact_id(&args.id)?,
        expected_revision: args.revision,
    };
    let mut result = write(memory, actor, change).await?;
    result["limits"] = serde_json::json!(FORGET_LIMITS);
    Ok(result)
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
