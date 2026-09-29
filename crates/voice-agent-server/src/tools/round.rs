//! The Tool-round Executor's own vocabulary: what one round may do, when a turn runs out of
//! budget, and what a call's outcome becomes on the wire.
//!
//! Everything here is pure and synchronous.  The executor that *uses* it is owned by a Voice
//! Session, but the decisions that make a round bounded — how many calls it may contain, how long
//! the turn may spend, and which bounded class a failure is reported as — are the same regardless
//! of which transport a call took, so they belong together and away from the actor.
//!
//! Two rules shape every function in this module:
//!
//! - A cap, an exhausted budget and a cancellation are *turn-level* outcomes.  They are reported
//!   as [`ToolRoundFailure`] and terminalize the turn, because none of them is the outcome of a
//!   completed ToolCall and none may become a synthetic ToolResult standing in for one.
//! - A call that did complete always produces exactly one ToolResult, at the index of the call it
//!   belongs to, whether it succeeded or failed.  [`external_tool_result`] is the only place an
//!   External Tool Call's outcome becomes content, and it can carry no remote body, URL, argument,
//!   credential or internal diagnostic.

use std::time::Duration;

use super::external_mcp::{ExternalMcpError, ExternalToolOutcome};

/// The three policy caps the Tool-round Executor runs under.
///
/// These arrive already validated from the configuration layer, which is the only place their
/// bounds are checked: a session that holds one of these has a deployment that either started or
/// failed to start, so nothing downstream re-derives or re-checks them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ToolRoundLimits {
    /// How many ToolCalls one LLM round may contain.  Checked against the whole round before call
    /// one, so a round that exceeds it executes nothing at all.
    pub max_calls_per_round: usize,
    /// How many tool rounds one Conversational Turn may continue past its first.
    pub max_rounds_per_turn: usize,
    /// The Tool Execution Budget: what one turn may spend on tool work in total, starting at its
    /// first ToolCall.
    pub execution_budget: Duration,
}

impl ToolRoundLimits {
    pub fn from_config(config: &crate::config::LlmToolsConfig) -> Self {
        Self {
            max_calls_per_round: config.max_calls_per_round,
            max_rounds_per_turn: config.max_rounds_per_turn,
            execution_budget: Duration::from_millis(config.execution_budget_ms),
        }
    }
}

impl Default for ToolRoundLimits {
    fn default() -> Self {
        Self::from_config(&crate::config::LlmToolsConfig::default())
    }
}

/// Why a Conversational Turn stopped without continuing.
///
/// Each of these is reached before the work it bounds could have started, so none of them has a
/// matching completed ToolCall to report as a result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolRoundFailure {
    /// The round asked for more calls than `max_calls_per_round` allows.  No call in the round ran.
    CallLimitExceeded,
    /// The turn has continued as many tool rounds as `max_rounds_per_turn` allows.  The check runs
    /// before the next round's request is sent, so no further call starts.
    RoundLimitExceeded,
    /// The turn's Tool Execution Budget ran out.  Whether it ran out before the next call or while
    /// one was in flight, the outcome for the model is the same — a turn with no tool time left —
    /// so it is one class, reported as one bounded string.
    ExecutionBudgetExceeded,
}

impl ToolRoundFailure {
    /// The single bounded spelling this failure is logged and reported as.
    pub fn code(self) -> &'static str {
        match self {
            Self::CallLimitExceeded => "tool_call_limit_exceeded",
            Self::RoundLimitExceeded => "tool_round_limit_exceeded",
            Self::ExecutionBudgetExceeded => "tool_execution_budget_exceeded",
        }
    }
}

/// The outcome of one External Tool Call, as it becomes ToolResult content.
///
/// A success keeps the same `ok`/`code`/`content`/`truncated` envelope a Device MCP result is
/// normalized into, so one turn's continuation reads one shape whichever transport produced it.  A
/// failure carries the bounded class and nothing else: no remote body, exception, URL, arguments,
/// secret or internal diagnostic ever reaches the model.
pub fn external_tool_result(outcome: Result<&ExternalToolOutcome, ExternalMcpError>) -> String {
    let (ok, code, content) = match outcome {
        Ok(success) => (
            !success.is_error,
            success.is_error.then_some("external_tool_error"),
            success.content.as_str(),
        ),
        Err(error) => return external_tool_error_result(error),
    };
    serde_json::json!({
        "ok": ok,
        "code": code,
        "content": content,
        "truncated": false,
    })
    .to_string()
}

/// A failure's ToolResult: the bounded class alone, generated here and nowhere else.
pub fn external_tool_error_result(error: ExternalMcpError) -> String {
    serde_json::json!({ "error": external_tool_error_class(error) }).to_string()
}

/// The bounded class one External Tool Call failure is reported and counted as.
///
/// `call_tool` produces exactly the five classes named here, and the mapping is total over the
/// whole error type, so a value can never fall through to a default spelling.  Anything else is
/// reported as a protocol fault: that is what a JSON-RPC error, or a discovery class that somehow
/// reached a call, both look like from here — a server that answered with something a call cannot
/// use.  The class names only the caller's own outcome and never where it went or what it was
/// authorized with.
pub fn external_tool_error_class(error: ExternalMcpError) -> &'static str {
    match error {
        ExternalMcpError::ToolTimeout => "external_tool_timeout",
        ExternalMcpError::ToolUnavailable => "external_tool_unavailable",
        ExternalMcpError::ToolAuthFailed => "external_tool_auth_failed",
        ExternalMcpError::ToolInvalidResponse => "external_tool_invalid_response",
        _ => "external_tool_protocol_error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn success(content: &str, is_error: bool) -> ExternalToolOutcome {
        ExternalToolOutcome {
            is_error,
            content: content.to_owned(),
        }
    }

    #[test]
    fn every_tool_call_failure_has_exactly_one_bounded_class() {
        let classes = [
            external_tool_error_class(ExternalMcpError::ToolTimeout),
            external_tool_error_class(ExternalMcpError::ToolUnavailable),
            external_tool_error_class(ExternalMcpError::ToolAuthFailed),
            external_tool_error_class(ExternalMcpError::ToolInvalidResponse),
            external_tool_error_class(ExternalMcpError::ToolProtocolError),
        ];
        assert_eq!(
            classes.as_slice(),
            [
                "external_tool_timeout",
                "external_tool_unavailable",
                "external_tool_auth_failed",
                "external_tool_invalid_response",
                "external_tool_protocol_error",
            ]
        );
    }

    /// A remote authentication refusal is a fact about one call.  It must not become a secret
    /// refresh, a catalog mutation or a session close, and the result says only the class.
    #[test]
    fn a_remote_refusal_carries_its_class_and_nothing_else() {
        let content = external_tool_result(Err(ExternalMcpError::ToolAuthFailed));
        assert_eq!(content, r#"{"error":"external_tool_auth_failed"}"#);
        for forbidden in [
            "401",
            "403",
            "http",
            "mcp.internal.test",
            "bearer",
            "token",
            "args",
        ] {
            assert!(
                !content.to_lowercase().contains(forbidden),
                "{content} must not mention {forbidden}"
            );
        }
    }

    /// A success keeps the envelope a Device MCP result is normalized into, so a turn that mixed
    /// both origins still reads one shape at the continuation.
    #[test]
    fn a_success_shares_the_device_result_envelope() {
        let content = external_tool_result(Ok(&success("21 degrees", false)));
        let parsed: serde_json::Value = serde_json::from_str(&content).expect("valid JSON");
        assert_eq!(parsed["ok"], true);
        assert_eq!(parsed["code"], serde_json::Value::Null);
        assert_eq!(parsed["content"], "21 degrees");
        assert_eq!(parsed["truncated"], false);
    }

    /// A server that answers `isError` is a completed call that failed, so it stays on the
    /// successful path: a paired AssistantToolCall/ToolResult, not a synthetic error envelope.
    #[test]
    fn a_remote_is_error_stays_a_paired_result() {
        let content = external_tool_result(Ok(&success("no such room", true)));
        let parsed: serde_json::Value = serde_json::from_str(&content).expect("valid JSON");
        assert_eq!(parsed["ok"], false);
        assert_eq!(parsed["code"], "external_tool_error");
        assert_eq!(parsed["content"], "no such room");
    }

    /// The caps and the budget are turn-level failures, so none of them may be spelled as a
    /// tool result that would stand in for a call that never completed.
    #[test]
    fn a_cap_or_budget_is_reported_as_a_turn_failure_and_never_as_content() {
        assert_eq!(
            ToolRoundFailure::CallLimitExceeded.code(),
            "tool_call_limit_exceeded"
        );
        assert_eq!(
            ToolRoundFailure::RoundLimitExceeded.code(),
            "tool_round_limit_exceeded"
        );
        assert_eq!(
            ToolRoundFailure::ExecutionBudgetExceeded.code(),
            "tool_execution_budget_exceeded"
        );
        for class in [
            ToolRoundFailure::CallLimitExceeded.code(),
            ToolRoundFailure::RoundLimitExceeded.code(),
            ToolRoundFailure::ExecutionBudgetExceeded.code(),
        ] {
            assert!(
                !external_tool_error_class(ExternalMcpError::ToolTimeout).contains(class),
                "a cap must not borrow a call failure's class"
            );
        }
    }

    /// The limits a session runs under are the deployment's, taken once: nothing downstream may
    /// re-derive or re-widen them.
    #[test]
    fn limits_come_from_the_validated_configuration_unchanged() {
        let config = crate::config::LlmToolsConfig {
            max_calls_per_round: 32,
            max_rounds_per_turn: 8,
            execution_budget_ms: 120_000,
        };
        assert_eq!(
            ToolRoundLimits::from_config(&config),
            ToolRoundLimits {
                max_calls_per_round: 32,
                max_rounds_per_turn: 8,
                execution_budget: Duration::from_millis(120_000),
            }
        );
        let default = ToolRoundLimits::default();
        assert_eq!(default.max_calls_per_round, 8);
        assert_eq!(default.max_rounds_per_turn, 4);
        assert_eq!(default.execution_budget, Duration::from_millis(30_000));
    }
}
