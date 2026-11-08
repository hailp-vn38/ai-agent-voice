//! Read-only Wiki builtins. Queries and page text live in the arguments and results, so neither is
//! ever logged: the lines here carry the tool, the call, the session and a result code.

use super::*;

use crate::tools::{
    builtin::{
        MAX_WIKI_SEARCH_LIMIT, WIKI_READ_TOOL_NAME, WIKI_SEARCH_TOOL_NAME, parse_wiki_read_args,
        parse_wiki_search_args,
    },
    knowledge_mcp::{
        KnowledgeMcp, KnowledgeMcpAccess, KnowledgeToolContext, KnowledgeToolError,
        bound_result_chars,
    },
};
use serde_json::{Value, json};
use std::sync::Arc;

/// A lookup the user is waiting on out loud; anything slower belongs in a Task.
const WIKI_CALL_BUDGET: Duration = Duration::from_secs(5);
const WIKI_RESULT_MAX_CHARS: usize = 6_000;
const WIKI_READ_MAX_LINES: usize = 80;

impl SessionActor {
    pub(in super::super) fn execute_wiki_tool(&mut self, call: ToolCall) {
        let Some(wiki) = self.session_wiki.clone() else {
            self.reject_wiki_tool(call, "wiki_disabled");
            return;
        };
        let Some(turn) = self.turn.as_ref() else {
            self.reject_wiki_tool(call, "no_active_turn");
            return;
        };
        let (turn_id, generation) = (turn.turn_id, turn.generation);
        let cancellation = turn.cancellation.clone();
        let budget = self
            .tool_rounds
            .remaining()
            .unwrap_or(Duration::ZERO)
            .min(WIKI_CALL_BUDGET);
        if budget.is_zero() {
            self.reject_wiki_tool(call, "wiki_timeout");
            return;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            self.reject_wiki_tool(call, "wiki_unavailable");
            return;
        };
        tracing::info!(
            event = "voice_wiki_tool_called",
            session_id = %self.session_id,
            turn_id = turn_id.get(),
            tool = %call.name,
            call_id = %call.id,
        );
        let context = KnowledgeToolContext {
            access: KnowledgeMcpAccess::ReadOnly,
            cancellation,
            deadline: tokio::time::Instant::now() + budget,
        };
        let completions = self.async_calls_tx.clone();
        let session_id = self.session_id.clone();
        let dispatched = call.clone();
        runtime.spawn(async move {
            let outcome = run_wiki_tool(&wiki, &context, &dispatched).await;
            // A barge-in owns the turn now, and nothing was written that needs a result.
            if context.cancellation.is_cancelled() {
                return;
            }
            tracing::info!(
                event = "voice_wiki_tool_completed",
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

    fn reject_wiki_tool(&mut self, call: ToolCall, code: &'static str) {
        tracing::warn!(
            event = "voice_wiki_tool_rejected",
            session_id = %self.session_id,
            tool = %call.name,
            call_id = %call.id,
            code,
        );
        self.complete_tool_call(call, Err(code));
    }
}

/// Lexical search needs no embedder and no reranker, which keeps a spoken lookup fast. The error
/// is the tool result's code; engine messages can hold vault paths, so they stay out of it.
async fn run_wiki_tool(
    wiki: &Arc<KnowledgeMcp>,
    context: &KnowledgeToolContext,
    call: &ToolCall,
) -> Result<Value, &'static str> {
    let (tool, arguments) = match call.name.as_str() {
        WIKI_SEARCH_TOOL_NAME => {
            let args = parse_wiki_search_args(&call.arguments).map_err(|_| "invalid_arguments")?;
            let limit = args
                .limit
                .unwrap_or(MAX_WIKI_SEARCH_LIMIT)
                .min(MAX_WIKI_SEARCH_LIMIT);
            (
                "wiki_query",
                json!({
                    "searches": [{"type": "lex", "query": args.query}],
                    "collections": ["wiki"],
                    "limit": limit,
                    "rerank": false,
                }),
            )
        }
        WIKI_READ_TOOL_NAME => {
            let args = parse_wiki_read_args(&call.arguments).map_err(|_| "invalid_arguments")?;
            (
                "wiki_get",
                json!({
                    "file": args.file,
                    "from_line": args.from_line,
                    "max_lines": WIKI_READ_MAX_LINES,
                    "line_numbers": false,
                }),
            )
        }
        _ => return Err("wiki_tool_unknown"),
    };
    let found = wiki
        .call_tool(context, tool, arguments)
        .await
        .map_err(error_code)?;
    let mut result = bound_result_chars(found, WIKI_RESULT_MAX_CHARS);
    result["untrusted_data"] = json!(true);
    Ok(result)
}

fn error_code(error: KnowledgeToolError) -> &'static str {
    match error {
        KnowledgeToolError::Denied => "wiki_tool_denied",
        KnowledgeToolError::InvalidArguments(_) => "invalid_arguments",
        KnowledgeToolError::ExecutionFailed(_) => "wiki_lookup_failed",
        KnowledgeToolError::Unavailable => "wiki_unavailable",
        KnowledgeToolError::Cancelled => "wiki_cancelled",
        KnowledgeToolError::Timeout => "wiki_timeout",
        KnowledgeToolError::ReadOnly => "wiki_read_only",
    }
}
