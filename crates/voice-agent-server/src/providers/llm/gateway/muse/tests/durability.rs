use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::harness::{
    FakeMuseVmTransport, FakeVmAccount, SINGLE_COOKIE, build_gateway_muse_provider,
    mp4_with_duration,
};
use super::media::{full_video_turn, single_video_history};
use crate::providers::llm::gateway::LlmGatewayProvider;
use crate::providers::llm::{
    DurableMediaProvider, LlmError, LlmOperation, OperationJournal, OperationState,
};
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

fn video_request(duration_secs: u32, continuation_key: Option<&str>) -> LlmOperation {
    LlmOperation::VideoGeneration {
        prompt: "story".into(),
        width: 640,
        height: 360,
        duration_secs,
        first_frame: None,
        continuation_key: continuation_key.map(str::to_string),
    }
}

/// A client-generated session has no remote turns yet, so its baseline must not depend on reading
/// a history that Muse may not serve until the first turn arrives.
#[tokio::test]
async fn fresh_session_submits_when_history_is_unreadable_before_first_turn() {
    let acc = FakeVmAccount::new(vec![full_video_turn("sandbox:///home/user/video.mp4")])
        .with_file("video.mp4", mp4_with_duration(30))
        .with_history_unavailable_before_first_turn();
    let provider = build_gateway_muse_provider(
        &[SINGLE_COOKIE],
        FakeMuseVmTransport::new(vec![acc.clone()]),
    )
    .await;
    let journal = Journal::default();
    let result = provider
        .submit(&video_request(30, None), None, &journal)
        .await;
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(acc.stream_requests().len(), 1);
    let updates = journal.0.lock().unwrap();
    let pending = updates
        .iter()
        .find(|u| u.status == Some(ToolOperationStatus::Submitting))
        .expect("the pending turn must be journaled before sending");
    assert_eq!(pending.user_turns_before, Some(0));
    assert!(pending.remote_session_id.is_some());
}

/// Nothing was sent, so this is a plain provider error instead of an unknown outcome that blocks
/// the session until an operator resolves it.
#[tokio::test]
async fn reused_session_history_failure_before_send_is_not_outcome_unknown() {
    let acc = FakeVmAccount::new(vec![full_video_turn("sandbox:///home/user/video.mp4")])
        .with_file("video.mp4", mp4_with_duration(30));
    let provider = build_gateway_muse_provider(
        &[SINGLE_COOKIE],
        FakeMuseVmTransport::new(vec![acc.clone()]),
    )
    .await;
    let request = video_request(30, Some("history-failure-job"));
    provider
        .submit(&request, None, &Journal::default())
        .await
        .unwrap();
    acc.set_history_failure_status(503);

    let journal = Journal::default();
    let result = provider.submit(&request, None, &journal).await;

    assert!(matches!(result, Err(LlmError::Transient)), "{result:?}");
    assert_eq!(acc.stream_requests().len(), 1, "no second turn may be sent");
    assert!(
        journal.0.lock().unwrap().is_empty(),
        "an unsent turn must not leave a submission baseline"
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
        SharedMusePool, release_recorded_job_session, restore_job_sessions_from_operations,
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
        let _ = release_recorded_job_session(&summary.operation).await;
    }
    let mut deleted = acc.deleted_sessions();
    deleted.sort();
    assert_eq!(
        deleted,
        ["newer-session", "older-session"],
        "each operation's remote session must be cleaned even when the registry holds only the latest one"
    );
}

/// Lớp đo của spike thật: đọc được history và file bằng kết nối mới, không cần stream cũ.
#[tokio::test]
async fn spike_probe_reads_history_and_file_without_the_original_stream() {
    use super::super::pool::SharedMusePool;
    use super::super::workflow::live_spike::probe_session;

    let acc = FakeVmAccount::new(vec![]).with_file("/tmp/spike.mp4", mp4_with_duration(8));
    acc.set_history_polls(single_video_history("done", "/tmp/spike.mp4"));
    let transport = FakeMuseVmTransport::new(vec![acc.clone()]);
    let provider = build_gateway_muse_provider(&[SINGLE_COOKIE], transport.clone()).await;
    let lease = provider
        .hub
        .acquire_lease(&GatewayBinding::Pool(GatewayVendor::Muse))
        .unwrap();
    let pool = SharedMusePool::for_gateway_lease(
        &reqwest::Client::new(),
        &lease.account_key,
        lease.access_token.expose(),
        Some(transport),
    );

    let probe = probe_session(&pool, "spike-session", None).await;
    assert_eq!(probe.acquire_error, None);
    assert_eq!(probe.history_status, Some(200));
    assert_eq!(probe.ready_paths, ["/tmp/spike.mp4"]);
    assert_eq!(probe.read_path.as_deref(), Some("/tmp/spike.mp4"));
    assert!(probe.read_bytes.is_some_and(|bytes| bytes > 0));
    assert_eq!(probe.read_error, None);
    assert!(
        acc.deleted_sessions().is_empty(),
        "probing must never delete the session being measured"
    );
}

/// Gọi `render` ở lượt đầu, rồi kết thúc bằng text.
#[derive(Debug)]
struct RenderThenFinish(std::sync::atomic::AtomicUsize);

#[async_trait::async_trait]
impl orchestrator_agent_rs::CustomModelExecutor for RenderThenFinish {
    fn routing_fingerprint(&self) -> String {
        "test-render-then-finish".into()
    }

    async fn complete(
        &self,
        _request: &orchestrator_agent_rs::ModelRequest,
        _cancel: &tokio_util::sync::CancellationToken,
        _sink: orchestrator_agent_rs::StreamSink<'_>,
    ) -> Result<orchestrator_agent_rs::ModelResponse, orchestrator_agent_rs::GatewayError> {
        let first = self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0;
        Ok(orchestrator_agent_rs::ModelResponse {
            text: if first { String::new() } else { "done".into() },
            finish_reason: if first { "tool_calls" } else { "stop" }.into(),
            tool_calls: if first {
                vec![orchestrator_agent_rs::ModelToolCall {
                    id: "render-1".into(),
                    name: "render".into(),
                    arguments: serde_json::json!({}),
                }]
            } else {
                Vec::new()
            },
            usage: None,
        })
    }
}

/// Ghi operation của Job Session như `video_create`, rồi giữ run sống tới khi test cho phép.
struct RenderGate {
    account_key: String,
    recorded: tokio::sync::Notify,
    finish: tokio::sync::Notify,
}

struct RenderTool(std::sync::Arc<RenderGate>);

#[async_trait::async_trait]
impl orchestrator_agent_rs::HostTool for RenderTool {
    fn descriptor(&self) -> orchestrator_agent_rs::ToolDescriptor {
        orchestrator_agent_rs::ToolDescriptor {
            name: "render".into(),
            description: "holds the run open while a job session exists".into(),
            input_schema: serde_json::json!({ "type": "object", "properties": {} }),
            effect: orchestrator_agent_rs::ToolEffect::Resumable,
        }
    }

    async fn execute(
        &self,
        context: orchestrator_agent_rs::ToolContext,
        _args: serde_json::Value,
    ) -> Result<orchestrator_agent_rs::ToolOutcome, orchestrator_agent_rs::ToolError> {
        let operation = context.operation();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        operation.begin(now + 600, "muse").await?;
        operation
            .advance(OperationUpdate {
                status: Some(ToolOperationStatus::Running),
                account_key: Some(self.0.account_key.clone()),
                remote_session_id: Some("terminal-release-session".into()),
                continuation_key: Some(context.scope().root_session_id.clone()),
                ..Default::default()
            })
            .await?;
        self.0.recorded.notify_one();
        self.0.finish.notified().await;
        operation
            .advance(OperationUpdate {
                status: Some(ToolOperationStatus::Committed),
                result_json: Some("{}".into()),
                ..Default::default()
            })
            .await?;
        Ok(orchestrator_agent_rs::ToolOutcome {
            call_id: context.scope().tool_call_id.clone(),
            content: "rendered".into(),
            metadata: serde_json::json!({}),
        })
    }
}

/// `TaskTerminal` của hủy đến khi run còn chạy: lúc đó xóa session sẽ làm hỏng lượt đang render.
#[tokio::test]
async fn terminal_release_waits_for_the_active_run_then_deletes_the_session_once() {
    use super::super::pool::{SharedMusePool, restore_job_sessions_from_operations};
    use crate::services::task_flow::release_terminal_job_sessions;
    use orchestrator_agent_rs::{
        AgentDefinition, AgentMode, CallerContext, LlmBackend, LlmConfig, LlmProviderConfig,
        Orchestrator, OrchestratorConfig, ProviderAuth, RequestLimits, RunStatus,
    };

    let acc = FakeVmAccount::new(vec![]);
    let transport = FakeMuseVmTransport::new(vec![acc.clone()]);
    let provider = build_gateway_muse_provider(&[SINGLE_COOKIE], transport.clone()).await;
    let lease = provider
        .hub
        .acquire_lease(&GatewayBinding::Pool(GatewayVendor::Muse))
        .unwrap();
    let _pool = SharedMusePool::for_gateway_lease(
        &reqwest::Client::new(),
        &lease.account_key,
        lease.access_token.expose(),
        Some(transport),
    );

    let dir = std::env::temp_dir().join(format!("terminal-release-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let prompt = dir.join("root.md");
    std::fs::write(&prompt, "test root").unwrap();
    let gate = std::sync::Arc::new(RenderGate {
        account_key: lease.account_key.clone(),
        recorded: tokio::sync::Notify::new(),
        finish: tokio::sync::Notify::new(),
    });
    let orchestrator = Orchestrator::builder(OrchestratorConfig::new(
        LlmConfig {
            providers: vec![LlmProviderConfig {
                id: "test".into(),
                backend: LlmBackend::OpenAi,
                endpoint: None,
                auth: ProviderAuth::CustomExecutor(std::sync::Arc::new(RenderThenFinish(
                    Default::default(),
                ))),
                limits: RequestLimits::uniform(Duration::from_secs(10)),
            }],
        },
        vec![
            AgentDefinition::new("root", "test", "model")
                .with_mode(AgentMode::Primary)
                .with_prompt_path(&prompt)
                .with_allowed_tools(["render"]),
        ],
        dir.join("store"),
    ))
    .register_tool(std::sync::Arc::new(RenderTool(gate.clone())))
    .build()
    .await
    .unwrap();

    let caller = CallerContext::new("alice", "ws1");
    let session_id = orchestrator.create_session(&caller, "root").await.unwrap();
    let turn = orchestrator
        .submit(&caller, &session_id, "render it".into())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(20), gate.recorded.notified())
        .await
        .expect("tool must record its operation");

    assert!(
        !release_terminal_job_sessions(&orchestrator, &session_id).await,
        "an active run must ask the caller to retry"
    );
    assert!(acc.deleted_sessions().is_empty());

    gate.finish.notify_one();
    tokio::time::timeout(Duration::from_secs(20), async {
        while orchestrator
            .result(&caller, &session_id, &turn.turn_id)
            .await
            .unwrap()
            .status
            == RunStatus::Running
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("run must finish");

    let ops = orchestrator.list_tool_operations().await.unwrap();
    restore_job_sessions_from_operations(&ops).await;
    assert!(release_terminal_job_sessions(&orchestrator, &session_id).await);
    assert_eq!(acc.deleted_sessions(), ["terminal-release-session"]);
    let ops = orchestrator.list_tool_operations().await.unwrap();
    assert!(ops[0].operation.remote_deleted_at.is_some());

    orchestrator.shutdown().await.unwrap();
    let _ = std::fs::remove_dir_all(&dir);
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

fn turn_with_chat_error(message: &str) -> super::harness::FakeStreamTurn {
    super::harness::FakeStreamTurn::media_reply(
        vec![],
        vec![],
        vec![serde_json::json!({ "event": "chat.error", "payload": { "message": message } })],
    )
}

fn glitch_history() -> Vec<Vec<serde_json::Value>> {
    vec![vec![
        serde_json::json!({ "role": "user", "display_text": "prompt" }),
        serde_json::json!({ "role": "assistant", "status": "completed", "display_text": "We hit a glitch" }),
    ]]
}

async fn settle_spawned_cleanup() {
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
}

/// Lỗi của lượt đang chạy phải giữ session để attempt sau nối lại; chỉ khi job kết thúc mới xóa.
async fn assert_running_failure_keeps_job_session(
    turn: super::harness::FakeStreamTurn,
    job_key: &str,
    is_expected: fn(&LlmError) -> bool,
) {
    use super::super::pool::{SharedMusePool, release_job_session};
    let acc = FakeVmAccount::new(vec![turn]);
    let transport = FakeMuseVmTransport::new(vec![acc.clone()]);
    let provider = build_gateway_muse_provider(&[SINGLE_COOKIE], transport.clone()).await;
    let lease = provider
        .hub
        .acquire_lease(&GatewayBinding::Pool(GatewayVendor::Muse))
        .unwrap();
    let _pool = SharedMusePool::for_gateway_lease(
        &reqwest::Client::new(),
        &lease.account_key,
        lease.access_token.expose(),
        Some(transport),
    );

    let result = provider
        .submit(&video_request(30, Some(job_key)), None, &Journal::default())
        .await;

    assert!(
        result.as_ref().is_err_and(is_expected),
        "unexpected result: {result:?}"
    );
    settle_spawned_cleanup().await;
    assert_eq!(acc.stream_requests().len(), 1);
    assert!(
        acc.deleted_sessions().is_empty(),
        "the running turn must survive for a later attempt: {:?}",
        acc.deleted_sessions()
    );

    let session_id = acc.stream_requests()[0]["session_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        release_job_session(job_key).await.as_deref(),
        Some(session_id.as_str()),
        "the job must still own the session it retained"
    );
    assert_eq!(acc.deleted_sessions(), [session_id]);
}

#[tokio::test]
async fn chat_error_on_running_turn_keeps_job_session() {
    assert_running_failure_keeps_job_session(
        turn_with_chat_error("stream interrupted"),
        "running-chat-error",
        |err| matches!(err, LlmError::Transient),
    )
    .await;
}

#[tokio::test]
async fn glitch_reply_on_running_turn_keeps_job_session() {
    let turn = super::harness::FakeStreamTurn::media_reply(vec![], glitch_history(), vec![]);
    assert_running_failure_keeps_job_session(turn, "running-glitch", |err| {
        matches!(err, LlmError::Transient)
    })
    .await;
}

#[tokio::test]
async fn quota_error_on_running_turn_keeps_job_session() {
    assert_running_failure_keeps_job_session(
        turn_with_chat_error("You reached your limit"),
        "running-quota",
        |err| matches!(err, LlmError::RateLimited { .. }),
    )
    .await;
}

#[tokio::test]
async fn glitch_reply_while_resuming_keeps_job_session() {
    let acc = FakeVmAccount::new(vec![]);
    acc.set_history_polls(glitch_history());
    let provider = build_gateway_muse_provider(
        &[SINGLE_COOKIE],
        FakeMuseVmTransport::new(vec![acc.clone()]),
    )
    .await;
    let mut resumed = state(operation(&provider, 0), 30);
    resumed.continuation_key = Some("resume-glitch".into());

    let result = provider.resume(&resumed, &Journal::default()).await;

    assert!(matches!(result, Err(LlmError::Transient)), "{result:?}");
    settle_spawned_cleanup().await;
    assert!(
        acc.deleted_sessions().is_empty(),
        "resume must keep the session for the next attempt: {:?}",
        acc.deleted_sessions()
    );
}

#[tokio::test]
async fn running_failure_without_job_still_deletes_the_session() {
    let acc = FakeVmAccount::new(vec![turn_with_chat_error("stream interrupted")]);
    let provider = build_gateway_muse_provider(
        &[SINGLE_COOKIE],
        FakeMuseVmTransport::new(vec![acc.clone()]),
    )
    .await;

    let result = provider
        .submit(&video_request(30, None), None, &Journal::default())
        .await;

    assert!(matches!(result, Err(LlmError::Transient)), "{result:?}");
    settle_spawned_cleanup().await;
    assert_eq!(acc.deleted_sessions().len(), 1);
}

/// Nhánh này xóa slot nên không còn ai dọn session, kể cả với lease đang retain.
#[tokio::test]
async fn failure_before_the_turn_runs_deletes_the_abandoned_job_session() {
    use super::super::pool::release_job_session;
    let mut rejected = turn_with_chat_error("unused");
    rejected.status = 429;
    let acc = FakeVmAccount::new(vec![
        full_video_turn("sandbox:///home/user/video.mp4"),
        rejected,
    ])
    .with_file("video.mp4", mp4_with_duration(30));
    let provider = build_gateway_muse_provider(
        &[SINGLE_COOKIE],
        FakeMuseVmTransport::new(vec![acc.clone()]),
    )
    .await;
    let request = video_request(30, Some("abandoned-before-running"));
    provider
        .submit(&request, None, &Journal::default())
        .await
        .unwrap();

    let result = provider.submit(&request, None, &Journal::default()).await;

    assert!(
        matches!(result, Err(LlmError::RateLimited { .. })),
        "{result:?}"
    );
    settle_spawned_cleanup().await;
    let session_id = acc.stream_requests()[0]["session_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(acc.deleted_sessions(), [session_id]);
    assert_eq!(release_job_session("abandoned-before-running").await, None);
}
