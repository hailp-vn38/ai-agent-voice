use super::*;
use crate::{
    config::McpResultDelivery,
    providers::{
        LlmError, LlmProvider,
        llm::{ChatMessage, LlmRequest},
    },
    services::background::VoiceJobSubmit,
    tools::builtin::CREATE_JOB_TOOL_NAME,
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
        Ok("Job queued".into())
    }
}
fn actor() -> SessionActor {
    let mut actor = super::tests::admitted_actor(&super::tests::catalog());
    actor.job_submit = VoiceJobSubmit::admitted(None, 1, 2);
    actor
}
fn call() -> ToolCall {
    ToolCall {
        id: "job-call".into(),
        name: CREATE_JOB_TOOL_NAME.into(),
        arguments: serde_json::json!({"goal":"Research public AI discussion"}),
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
                    .filter(|tool| tool.name == CREATE_JOB_TOOL_NAME)
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
async fn async_job_receipt_matches_original_tool_round_and_survives_abort_after_commit() {
    let (db, _, runtime) = crate::services::background::tests::harness().await;
    let requester = crate::services::background::tests::voice(&db).await;
    let crate::services::background::JobRequester::Voice {
        device_incarnation,
        agent_incarnation,
    } = requester
    else {
        unreachable!()
    };
    let mut actor = actor();
    actor.job_submit = VoiceJobSubmit::admitted(
        Some(runtime.engine.clone()),
        device_incarnation,
        agent_incarnation,
    );
    actor.llm_runtime = Arc::new(crate::workers::LlmRuntime::new(
        Arc::new(CapableLlm),
        2,
        Duration::from_secs(1),
    ));
    let (mailbox, mut events) = mpsc::channel(2);
    actor = actor.with_job_mailbox(mailbox.clone());
    actor.begin_active_turn().unwrap();
    actor
        .commit_user_text("Research in background".into())
        .unwrap();
    actor.start_tool_batch(vec![call()]);
    let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
        .await
        .unwrap()
        .unwrap();
    let SessionEvent::JobSubmitCompleted {
        session_id,
        turn_id,
        generation,
        tool_call_id,
        outcome,
    } = event
    else {
        panic!("expected Job receipt")
    };
    assert_eq!(outcome["created"], true);
    let id = outcome["job_id"].as_str().unwrap().to_owned();
    assert_eq!(
        runtime.engine.get(&id).await.unwrap().request.goal,
        "Research public AI discussion"
    );
    actor.complete_job_submit(
        session_id.clone(),
        turn_id,
        generation,
        tool_call_id.clone(),
        outcome.clone(),
    );
    let result = actor
        .llm_messages
        .iter()
        .find_map(|message| match message {
            ChatMessage::ToolResult {
                tool_call_id,
                content,
            } if tool_call_id == "job-call" => Some(content),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(result).unwrap(),
        outcome
    );
    actor.cancel_tool_turn();
    actor.complete_job_submit(session_id, turn_id, generation, tool_call_id, outcome);
    assert_eq!(runtime.engine.list(1, 30, None, None).await.unwrap().1, 1);
    runtime.shutdown().await;
}

#[tokio::test]
async fn job_submit_rejects_forged_identity_and_replays_committed_receipt_after_disable() {
    let (db, _, runtime) = crate::services::background::tests::harness().await;
    let requester = crate::services::background::tests::voice(&db).await;
    let crate::services::background::JobRequester::Voice {
        device_incarnation,
        agent_incarnation,
    } = requester
    else {
        unreachable!()
    };
    let handle = VoiceJobSubmit::admitted(
        Some(runtime.engine.clone()),
        device_incarnation,
        agent_incarnation,
    )
    .unwrap();
    let bad = handle
        .submit(
            serde_json::json!({"goal":"Bad identity","device_id":"forged"}),
            "invalid",
            Duration::from_secs(1),
        )
        .await;
    assert_eq!(bad["created"], false);
    assert_eq!(bad["isError"], true);
    assert_eq!(bad["error"]["code"], "invalid_arguments");
    let args = serde_json::json!({"goal":"Replay committed Voice Job"});
    let success = handle
        .submit(args.clone(), "voice-once", Duration::from_secs(1))
        .await;
    assert_eq!(success["created"], true);
    let settings = runtime.engine.settings().await.unwrap();
    runtime
        .engine
        .update_settings(
            crate::services::background::BackgroundSettingsUpdate {
                enabled: false,
                ..settings.settings
            },
            settings.revision,
        )
        .await
        .unwrap();
    let replay = handle
        .submit(args, "voice-once", Duration::from_secs(1))
        .await;
    assert_eq!(replay, success);
    let rejected = handle
        .submit(
            serde_json::json!({"goal":"Another Job"}),
            "disabled",
            Duration::from_secs(1),
        )
        .await;
    assert_eq!(rejected["created"], false);
    assert_eq!(rejected["error"]["code"], "background_disabled");
    runtime.shutdown().await;
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
        serde_json::json!({"created":true,"job_id":"late","status":"queued"}),
    );
    assert_eq!(actor.llm_messages.len(), count);
    assert!(actor.job_pending.is_none());
}
