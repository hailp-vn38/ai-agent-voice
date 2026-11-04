use super::*;
use crate::{
    config::McpResultDelivery,
    providers::{
        LlmError, LlmProvider,
        llm::{ChatMessage, LlmRequest},
    },
    services::task_flow::VoiceTaskSubmit,
    tools::builtin::TASK_CREATE_TOOL_NAME,
};
use std::sync::Arc;
struct CapableLlm;
impl LlmProvider for CapableLlm {
    fn adapter(&self) -> &'static str {
        "fixture"
    }
    fn supports_tools(&self) -> bool {
        true
    }
    fn complete(&self, _: &LlmRequest) -> Result<String, LlmError> {
        Ok("Task queued".into())
    }
}
fn actor() -> SessionActor {
    let mut actor = super::tests::admitted_actor(&super::tests::catalog());
    actor.job_submit = Some(VoiceTaskSubmit::unbacked_for_test());
    actor
}
fn call() -> ToolCall {
    ToolCall {
        id: "job-call".into(),
        name: TASK_CREATE_TOOL_NAME.into(),
        arguments: serde_json::json!({
            "title": "Research public AI discussion",
            "prompt": "Research public AI discussion"
        }),
    }
}

#[test]
fn job_builtin_is_advertised_without_mcp_and_forces_llm_receipt_delivery() {
    let mut actor = actor();
    actor.llm_runtime = Arc::new(crate::workers::LlmRuntime::new(
        Arc::new(CapableLlm),
        2,
        Duration::from_secs(1),
    ));
    for enabled in [false, true] {
        actor.mcp.enabled = enabled;
        for delivery in [McpResultDelivery::Silent, McpResultDelivery::DirectTts] {
            actor.mcp.result_delivery = delivery;
            assert_eq!(
                actor
                    .available_llm_tools()
                    .iter()
                    .filter(|tool| tool.name == TASK_CREATE_TOOL_NAME)
                    .count(),
                1
            );
            assert_eq!(
                actor.round_result_delivery(&[call()]),
                McpResultDelivery::LlmThenTts
            );
            assert!(actor.offers_answer_changing_tools());
        }
    }
}
#[tokio::test]
async fn stale_job_completion_cannot_finish_a_new_turn_and_missing_tool_llm_is_typed() {
    let mut actor = actor();
    assert!(
        !actor.offers_answer_changing_tools(),
        "a text-only provider must retain ordinary streaming speech"
    );
    let (mailbox, _events) = mpsc::channel(2);
    actor = actor.with_job_mailbox(mailbox.clone());
    actor.begin_active_turn().unwrap();
    actor
        .commit_user_text("Create background work".into())
        .unwrap();
    actor.start_tool_batch(vec![call()]);
    let error = actor
        .llm_messages
        .iter()
        .find_map(|message| match message {
            ChatMessage::ToolResult { content, .. } => {
                serde_json::from_str::<serde_json::Value>(content).ok()
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(error["isError"], true);
    assert_eq!(error["error"]["code"], "llm_tool_calls_unavailable");
    let turn = actor.current_turn_id().unwrap();
    let generation = actor.generation;
    actor.cancel_tool_turn();
    let count = actor.llm_messages.len();
    actor.complete_job_submit(
        actor.session_id.clone(),
        turn,
        generation,
        "job-call".into(),
        serde_json::json!({"created":true,"task_id":"late","status":"queued"}),
    );
    assert_eq!(actor.llm_messages.len(), count);
    assert!(actor.job_pending.is_none());
}
