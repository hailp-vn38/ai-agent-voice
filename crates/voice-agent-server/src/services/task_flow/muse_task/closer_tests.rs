use super::*;
use orchestrator_agent_rs::{RunStatus, RunSummary};

fn run(status: RunStatus) -> RunSummary {
    RunSummary {
        run_id: "run".into(),
        session_id: "child".into(),
        turn_id: "turn".into(),
        agent_id: "cloud_worker".into(),
        attempt: 0,
        provider_id: "test".into(),
        model_id: "test".into(),
        status,
        steps: 1,
        input_tokens: 0,
        output_tokens: 0,
        started_at: 0,
        finished_at: None,
        final_message_id: None,
        error: None,
        next_attempt_at: None,
        retry_reason: None,
    }
}

#[test]
fn muse_task_closer_closes_completed_but_preserves_recoverable_children() {
    assert_eq!(
        decide_closure(Some(&run(RunStatus::Completed)), true),
        ClosureDecision::Close
    );
    for status in [
        RunStatus::Running,
        RunStatus::WaitingRetry,
        RunStatus::WaitingUser,
        RunStatus::Interrupted,
        RunStatus::Failed,
        RunStatus::TimedOut,
    ] {
        assert_eq!(
            decide_closure(Some(&run(status)), true),
            ClosureDecision::Keep,
            "{status:?}"
        );
    }
    assert_eq!(
        decide_closure(Some(&run(RunStatus::Cancelled)), true),
        ClosureDecision::Close
    );
    assert_eq!(decide_closure(None, true), ClosureDecision::Keep);
    assert_eq!(decide_closure(None, false), ClosureDecision::Close);
}

#[test]
fn muse_task_closer_failure_policy_matches_pinned_library_contract() {
    // Independent, fixed 0.6.0 contract, not generated from the server's classifier.
    for code in [
        "TOOL_OUTCOME_UNKNOWN",
        "RECOVERY_BLOCKED",
        "BUDGET_EXHAUSTED",
        "CONTEXT_LIMIT",
        "MODEL_NOT_CONFIGURED",
        "PROVIDER_INVALID_REQUEST",
        "PROVIDER_AUTH_FAILED",
        "PROVIDER_CONFIG_CHANGED",
        "CANCELLED",
        "CHILD_CANCELLED",
    ] {
        let mut failed = run(RunStatus::Failed);
        failed.error = Some(orchestrator_agent_rs::ResultError {
            code: code.into(),
            message: "test".into(),
        });
        assert_eq!(
            decide_closure(Some(&failed), true),
            ClosureDecision::Close,
            "{code}"
        );
    }
    for code in [
        "TOOL_RETRY_LATER",
        "PROVIDER_UNAVAILABLE",
        "UNKNOWN",
        "STEP_LIMIT_REACHED",
        "",
    ] {
        assert!(
            child_failure_is_retryable(RunStatus::Failed, code),
            "{code}"
        );
    }
    let lock = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../Cargo.lock"));
    let package = lock
        .split("[[package]]")
        .find(|p| p.lines().any(|l| l == "name = \"orchestrator-agent\""))
        .unwrap();
    assert!(
        package.lines().any(|l| l == "version = \"0.6.0\""),
        "Review the private child_failure_is_retryable contract before upgrading orchestrator-agent"
    );
}
