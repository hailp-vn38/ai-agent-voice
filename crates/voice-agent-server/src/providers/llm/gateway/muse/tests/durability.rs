use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::harness::{
    FakeMuseVmTransport, FakeVmAccount, SINGLE_COOKIE, build_gateway_muse_provider,
    mp4_with_duration,
};
use super::media::{full_video_turn, single_video_history};
use crate::providers::llm::gateway::LlmGatewayProvider;
use crate::providers::llm::{DurableMediaProvider, LlmError, OperationJournal, OperationState};
use crate::services::llm_gateway::{GatewayBinding, GatewayVendor};
use orchestrator_agent_rs::{OperationUpdate, ToolOperation, ToolOperationStatus};

#[derive(Default)]
struct Journal(Mutex<Vec<OperationUpdate>>);
#[async_trait::async_trait]
impl OperationJournal for Journal {
    async fn record(&self, update: OperationUpdate) -> Result<(), LlmError> {
        self.0.lock().unwrap().push(update);
        Ok(())
    }
}

fn operation(provider: &LlmGatewayProvider, batch_index: u32) -> ToolOperation {
    let account_key = provider
        .hub
        .acquire_lease(&GatewayBinding::Pool(GatewayVendor::Muse))
        .unwrap()
        .account_key;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    ToolOperation {
        id: "durable-op".into(),
        invocation_id: "durable-inv".into(),
        provider_ref: Some("muse".into()),
        account_key: Some(account_key),
        remote_session_id: Some("durable-session".into()),
        continuation_key: None,
        batch_index,
        repairs_used: 0,
        batch_paths_json: None,
        user_turns_before: Some(0),
        request_json: None,
        status: ToolOperationStatus::Running,
        artifact_ids_json: None,
        result_json: None,
        remote_deleted_at: None,
        last_activity_at: now,
        deadline_at: now + 1700,
        created_at: now,
    }
}

fn state(operation: ToolOperation, duration_secs: u32) -> OperationState {
    OperationState {
        operation,
        prompt: "story".into(),
        width: 640,
        height: 360,
        duration_secs,
        first_frame: None,
        continuation_key: None,
    }
}

#[tokio::test]
async fn resume_respects_original_operation_deadline() {
    let acc = FakeVmAccount::new(vec![full_video_turn("sandbox:///home/user/late.mp4")])
        .with_file("late.mp4", mp4_with_duration(30));
    acc.set_history_polls(single_video_history(
        "done [ALL_MEDIA_ATTACHED]",
        "sandbox:///home/user/late.mp4",
    ));
    let provider = build_gateway_muse_provider(
        &[SINGLE_COOKIE],
        FakeMuseVmTransport::new(vec![acc.clone()]),
    )
    .await;
    let mut op = operation(&provider, 0);
    op.deadline_at = op.created_at - 1;
    tokio::time::pause();
    let start = tokio::time::Instant::now();
    let result = provider.resume(&state(op, 30), &Journal::default()).await;
    assert!(matches!(result, Err(LlmError::Timeout)), "{result:?}");
    assert!(start.elapsed() < Duration::from_secs(1));
    assert!(acc.stream_requests().is_empty());
}

#[tokio::test]
async fn resume_between_batches_sends_only_the_pending_batch() {
    let first = mp4_with_duration(60);
    let second = mp4_with_duration(30);
    let acc = FakeVmAccount::new(vec![full_video_turn("sandbox:///home/user/second.mp4")])
        .with_file("first.mp4", first.clone())
        .with_file("second.mp4", second.clone());
    acc.set_history_polls(single_video_history(
        "done [ALL_MEDIA_ATTACHED]",
        "sandbox:///home/user/first.mp4",
    ));
    let provider = build_gateway_muse_provider(
        &[SINGLE_COOKIE],
        FakeMuseVmTransport::new(vec![acc.clone()]),
    )
    .await;
    let mut op = operation(&provider, 1);
    op.status = ToolOperationStatus::Submitting;
    op.user_turns_before = Some(1);
    op.batch_paths_json = Some(serde_json::json!(["sandbox:///home/user/first.mp4"]).to_string());
    let output = provider
        .resume(&state(op, 90), &Journal::default())
        .await
        .unwrap();
    let (parts, _) = output.into_media_parts().unwrap();
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0].bytes, first);
    assert_eq!(
        parts[1].bytes, second,
        "resume must send the second batch before waiting"
    );
    let requests = acc.stream_requests();
    assert_eq!(
        requests.len(),
        1,
        "the completed first batch must not be regenerated"
    );
    assert!(
        requests[0]["items"][0]["text"]
            .as_str()
            .unwrap()
            .contains("7-9")
    );
}

#[tokio::test]
async fn restored_session_cleanup_reconnects_without_generating_media() {
    use super::super::pool::{
        SharedMusePool, release_job_session, restore_job_sessions_from_operations,
    };
    let acc = FakeVmAccount::new(vec![]);
    let transport = FakeMuseVmTransport::new(vec![acc.clone()]);
    let provider = build_gateway_muse_provider(&[SINGLE_COOKIE], transport.clone()).await;
    let mut op = operation(&provider, 0);
    op.continuation_key = Some("restored-cleanup-only".into());
    let lease = provider
        .hub
        .acquire_account_for_polling(op.account_key.as_deref().unwrap())
        .unwrap();
    let _pool = SharedMusePool::for_gateway_lease(
        &reqwest::Client::new(),
        &lease.account_key,
        lease.access_token.expose(),
        Some(transport),
    );
    restore_job_sessions_from_operations(&[orchestrator_agent_rs::ToolOperationSummary {
        operation: op,
        invocation_status: "completed".into(),
        root_session_id: "restored-cleanup-only".into(),
        root_has_active_work: false,
    }])
    .await;
    assert_eq!(
        release_job_session("restored-cleanup-only")
            .await
            .as_deref(),
        Some("durable-session")
    );
    assert_eq!(acc.deleted_sessions(), ["durable-session"]);
    assert!(acc.stream_requests().is_empty());
}

#[tokio::test]
async fn duration_repairs_are_journaled_per_batch() {
    use crate::providers::llm::LlmOperation;
    let acc = FakeVmAccount::new(vec![
        full_video_turn("sandbox:///home/user/short.mp4"),
        full_video_turn("sandbox:///home/user/first.mp4"),
        full_video_turn("sandbox:///home/user/short.mp4"),
        full_video_turn("sandbox:///home/user/second.mp4"),
    ])
    .with_file("short.mp4", mp4_with_duration(10))
    .with_file("first.mp4", mp4_with_duration(60))
    .with_file("second.mp4", mp4_with_duration(30));
    let provider = build_gateway_muse_provider(
        &[SINGLE_COOKIE],
        FakeMuseVmTransport::new(vec![acc.clone()]),
    )
    .await;
    let journal = Journal::default();
    provider
        .submit(
            &LlmOperation::VideoGeneration {
                prompt: "story".into(),
                width: 640,
                height: 360,
                duration_secs: 90,
                first_frame: None,
                continuation_key: None,
            },
            None,
            &journal,
        )
        .await
        .unwrap();
    assert_eq!(acc.stream_requests().len(), 4);
    let updates = journal.0.lock().unwrap();
    for index in [0, 1] {
        assert!(
            updates.iter().any(|u| u.batch_index == Some(index)
                && u.repairs_used == Some(1)
                && u.status == Some(ToolOperationStatus::Submitting)),
            "repair must be journaled before sending, separately for batch {index}"
        );
    }
    assert!(
        !updates
            .iter()
            .any(|u| u.repairs_used.is_some_and(|n| n > 1)),
        "the repair count must reset in the next batch"
    );
}

#[tokio::test]
async fn checkpoint_failure_prevents_remote_submission() {
    struct FailingJournal;
    #[async_trait::async_trait]
    impl OperationJournal for FailingJournal {
        async fn record(&self, _: OperationUpdate) -> Result<(), LlmError> {
            Err(LlmError::ToolOutcomeUnknown)
        }
    }
    let acc = FakeVmAccount::new(vec![]);
    let provider = build_gateway_muse_provider(
        &[SINGLE_COOKIE],
        FakeMuseVmTransport::new(vec![acc.clone()]),
    )
    .await;
    let result = provider
        .submit(
            &crate::providers::llm::LlmOperation::VideoGeneration {
                prompt: "story".into(),
                width: 640,
                height: 360,
                duration_secs: 30,
                first_frame: None,
                continuation_key: None,
            },
            None,
            &FailingJournal,
        )
        .await;
    assert!(
        matches!(result, Err(LlmError::ToolOutcomeUnknown)),
        "{result:?}"
    );
    assert!(
        acc.stream_requests().is_empty(),
        "a failed checkpoint must never trigger a new video"
    );
}

#[tokio::test]
async fn deadline_expiring_during_download_prevents_next_batch_submission() {
    let acc = FakeVmAccount::new(vec![full_video_turn("sandbox:///home/user/first.mp4")])
        .with_file("first.mp4", mp4_with_duration(60))
        .with_read_delay(Duration::from_secs(4));
    let provider = build_gateway_muse_provider(
        &[SINGLE_COOKIE],
        FakeMuseVmTransport::new(vec![acc.clone()]),
    )
    .await;
    let mut op = operation(&provider, 0);
    op.status = ToolOperationStatus::Submitting;
    op.deadline_at = op.created_at + 2;
    tokio::time::pause();
    let result = provider
        .submit(
            &crate::providers::llm::LlmOperation::VideoGeneration {
                prompt: "story".into(),
                width: 640,
                height: 360,
                duration_secs: 90,
                first_frame: None,
                continuation_key: None,
            },
            Some(&op),
            &Journal::default(),
        )
        .await;
    assert!(matches!(result, Err(LlmError::Timeout)), "{result:?}");
    assert_eq!(
        acc.stream_requests().len(),
        1,
        "deadline must be checked again before sending batch two"
    );
}

#[tokio::test]
async fn attachment_followup_is_journaled_before_sending() {
    use super::harness::FakeStreamTurn;
    let acc = FakeVmAccount::new(vec![
        FakeStreamTurn::chat_reply(vec!["done"], "done"),
        full_video_turn("sandbox:///home/user/final.mp4"),
    ])
    .with_file("final.mp4", mp4_with_duration(30));
    let provider = build_gateway_muse_provider(
        &[SINGLE_COOKIE],
        FakeMuseVmTransport::new(vec![acc.clone()]),
    )
    .await;
    let journal = Journal::default();
    tokio::time::pause();
    provider
        .submit(
            &crate::providers::llm::LlmOperation::VideoGeneration {
                prompt: "story".into(),
                width: 640,
                height: 360,
                duration_secs: 30,
                first_frame: None,
                continuation_key: None,
            },
            None,
            &journal,
        )
        .await
        .unwrap();
    assert_eq!(acc.stream_requests().len(), 2);
    assert!(
        journal
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|u| u.status == Some(ToolOperationStatus::Submitting)
                && u.request_json
                    .as_deref()
                    .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
                    .is_some_and(|request| request["attachment_followup_sent"] == true)),
        "attachment nudge must persist its one-shot flag before sending"
    );
}

#[tokio::test]
async fn cleanup_deletes_all_remote_sessions_of_one_job() {
    use super::super::pool::{
        SharedMusePool, release_operation_session, restore_job_sessions_from_operations,
    };
    let acc = FakeVmAccount::new(vec![]);
    let transport = FakeMuseVmTransport::new(vec![acc.clone()]);
    let provider = build_gateway_muse_provider(&[SINGLE_COOKIE], transport.clone()).await;
    let mut old = operation(&provider, 0);
    old.continuation_key = Some("multi-session-cleanup".into());
    old.remote_session_id = Some("older-session".into());
    let mut new = old.clone();
    new.remote_session_id = Some("newer-session".into());
    let lease = provider
        .hub
        .acquire_account_for_polling(old.account_key.as_deref().unwrap())
        .unwrap();
    let _pool = SharedMusePool::for_gateway_lease(
        &reqwest::Client::new(),
        &lease.account_key,
        lease.access_token.expose(),
        Some(transport),
    );
    let summaries = [old, new].map(|operation| orchestrator_agent_rs::ToolOperationSummary {
        operation,
        invocation_status: "completed".into(),
        root_session_id: "multi-session-cleanup".into(),
        root_has_active_work: false,
    });
    restore_job_sessions_from_operations(&summaries).await;
    for summary in &summaries {
        let _ = release_operation_session(&summary.operation).await;
    }
    let mut deleted = acc.deleted_sessions();
    deleted.sort();
    assert_eq!(
        deleted,
        ["newer-session", "older-session"],
        "each operation's remote session must be cleaned even when the registry holds only the latest one"
    );
}

#[tokio::test]
async fn lost_submission_ack_keeps_session_for_reconciliation_without_failover() {
    use super::harness::SECOND_COOKIE;
    let first = FakeVmAccount::new(vec![full_video_turn("sandbox:///home/user/video.mp4")])
        .with_lost_stream_ack();
    let second = FakeVmAccount::new(vec![]);
    let provider = build_gateway_muse_provider(
        &[SINGLE_COOKIE, SECOND_COOKIE],
        FakeMuseVmTransport::new(vec![first.clone(), second.clone()]),
    )
    .await;
    let mut op = operation(&provider, 0);
    op.continuation_key = Some("lost-ack-job".into());
    op.status = ToolOperationStatus::Submitting;
    let result = provider
        .submit(
            &crate::providers::llm::LlmOperation::VideoGeneration {
                prompt: "story".into(),
                width: 640,
                height: 360,
                duration_secs: 30,
                first_frame: None,
                continuation_key: op.continuation_key.clone(),
            },
            Some(&op),
            &Journal::default(),
        )
        .await;
    assert!(
        matches!(result, Err(LlmError::ToolOutcomeUnknown)),
        "lost ACK must be reconciled, not treated as failed/expired: {result:?}"
    );
    assert_eq!(first.stream_requests().len(), 1);
    assert!(second.stream_requests().is_empty());
    assert!(
        first.deleted_sessions().is_empty(),
        "the accepted remote operation must survive for reconciliation"
    );
}

#[tokio::test]
async fn repair_deadline_does_not_report_partial_video_as_success() {
    use super::harness::FakeStreamTurn;
    let stuck = FakeStreamTurn::media_reply(
        vec![],
        vec![vec![
            serde_json::json!({"role":"user","display_text":"repair"}),
            serde_json::json!({"role":"assistant", "status":"in_progress", "display_text":"rendering"}),
        ]],
        vec![],
    );
    let acc = FakeVmAccount::new(vec![
        full_video_turn("sandbox:///home/user/short.mp4"),
        stuck,
    ])
    .with_file("short.mp4", mp4_with_duration(10));
    let provider =
        build_gateway_muse_provider(&[SINGLE_COOKIE], FakeMuseVmTransport::new(vec![acc])).await;
    let mut op = operation(&provider, 0);
    op.deadline_at = op.created_at + 5;
    op.status = ToolOperationStatus::Submitting;
    tokio::time::pause();
    let result = provider
        .submit(
            &crate::providers::llm::LlmOperation::VideoGeneration {
                prompt: "story".into(),
                width: 640,
                height: 360,
                duration_secs: 60,
                first_frame: None,
                continuation_key: None,
            },
            Some(&op),
            &Journal::default(),
        )
        .await;
    assert!(
        matches!(result, Err(LlmError::Timeout)),
        "incomplete footage at the shared deadline is a timeout: {result:?}"
    );
}

#[tokio::test]
async fn legacy_between_batch_checkpoint_requires_reconciliation() {
    let acc = FakeVmAccount::new(vec![]).with_file("first.mp4", mp4_with_duration(60));
    acc.set_history_polls(single_video_history(
        "done [ALL_MEDIA_ATTACHED]",
        "sandbox:///home/user/first.mp4",
    ));
    let provider = build_gateway_muse_provider(
        &[SINGLE_COOKIE],
        FakeMuseVmTransport::new(vec![acc.clone()]),
    )
    .await;
    let mut op = operation(&provider, 1);
    op.batch_paths_json = Some(serde_json::json!(["sandbox:///home/user/first.mp4"]).to_string());
    let result = provider.resume(&state(op, 90), &Journal::default()).await;
    assert!(
        matches!(result, Err(LlmError::ToolOutcomeUnknown)),
        "ambiguous legacy checkpoint must not download batch one twice: {result:?}"
    );
    assert!(acc.stream_requests().is_empty());
}
