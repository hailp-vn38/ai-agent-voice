use super::*;

#[tokio::test]
async fn agent_task_gateway_two_sends_share_session_and_collect_does_not_send() {
    let mut second = FakeStreamTurn::chat_reply(vec![], "Second [MUSE_TASK_DONE]");
    second.history_polls[0].insert(0, serde_json::json!({"role":"user"}));
    let account = FakeVmAccount::new(vec![
        FakeStreamTurn::chat_reply(vec![], "First [MUSE_TASK_DONE]"),
        second,
    ])
    .with_history_unavailable_before_first_turn();
    let provider = build_gateway_muse_provider(
        &[SINGLE_COOKIE],
        FakeMuseVmTransport::new(vec![account.clone()]),
    )
    .await;
    let mut req = request(&format!("muse_task:{}", uuid::Uuid::new_v4()));
    let journal = Journal::default();
    provider
        .agent_task()
        .unwrap()
        .run_turn(&req, &journal, &|_| {})
        .await
        .unwrap();
    assert_eq!(
        provider
            .agent_task()
            .unwrap()
            .run_turn(&req, &Journal::default(), &|_| {})
            .await
            .unwrap()
            .turn,
        2
    );
    let requests = account.stream_requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0]["session_id"], requests[1]["session_id"]);
    req.action = AgentTurnAction::Collect {
        output: AgentOutputKind::Message,
        filename: None,
    };
    assert_eq!(
        provider
            .agent_task()
            .unwrap()
            .run_turn(&req, &Journal::default(), &|_| {})
            .await
            .unwrap()
            .message,
        "Second"
    );
    assert_eq!(account.stream_requests().len(), 2);
}

#[tokio::test]
async fn agent_task_gateway_slice_timeout_and_stall_preserve_partial() {
    use crate::providers::llm::AgentTaskError;
    for (slice, deadline, expected) in [
        (5, 3600, "slice"),
        (1700, 5, "timeout"),
        (1700, 3600, "stall"),
    ] {
        let mut turn = FakeStreamTurn::chat_reply(vec![], "Partial report");
        turn.history_polls[0][1]["status"] = serde_json::json!("streaming");
        let account = FakeVmAccount::new(vec![turn]);
        let provider = build_gateway_muse_provider(
            &[SINGLE_COOKIE],
            FakeMuseVmTransport::new(vec![account.clone()]),
        )
        .await;
        let mut req = request(&format!("muse_task:{}", uuid::Uuid::new_v4()));
        req.wait_slice = Duration::from_secs(slice);
        req.operation.deadline_at = req.operation.created_at + deadline;
        tokio::time::pause();
        let error = provider
            .agent_task()
            .unwrap()
            .run_turn(&req, &Journal::default(), &|_| {})
            .await
            .unwrap_err();
        tokio::time::resume();
        match expected {
            "slice" => assert!(matches!(error, AgentTaskError::RetryLater(_))),
            "timeout" => assert_eq!(
                error,
                AgentTaskError::Timeout {
                    partial: Some("Partial report".into())
                }
            ),
            _ => assert_eq!(
                error,
                AgentTaskError::Stalled {
                    partial: Some("Partial report".into())
                }
            ),
        }
        assert_eq!(account.stream_requests().len(), 1);
        assert!(account.deleted_sessions().is_empty());
    }
}

#[tokio::test]
async fn agent_task_gateway_collect_without_session_does_not_post() {
    let account = FakeVmAccount::new(vec![]);
    let provider = build_gateway_muse_provider(
        &[SINGLE_COOKIE],
        FakeMuseVmTransport::new(vec![account.clone()]),
    )
    .await;
    let mut req = request(&format!("muse_task:{}", uuid::Uuid::new_v4()));
    req.action = AgentTurnAction::Collect {
        output: AgentOutputKind::Message,
        filename: None,
    };
    assert_eq!(
        provider
            .agent_task()
            .unwrap()
            .run_turn(&req, &Journal::default(), &|_| {})
            .await,
        Err(crate::providers::llm::AgentTaskError::NoSession)
    );
    assert!(account.stream_requests().is_empty());
}

#[tokio::test]
async fn agent_task_gateway_unwritable_checkpoint_prevents_post_and_preserves_unknown() {
    let account = FakeVmAccount::new(vec![]);
    let provider = build_gateway_muse_provider(
        &[SINGLE_COOKIE],
        FakeMuseVmTransport::new(vec![account.clone()]),
    )
    .await;
    assert_eq!(
        provider
            .agent_task()
            .unwrap()
            .run_turn(
                &request(&format!("muse_task:{}", uuid::Uuid::new_v4())),
                &UnwritableJournal,
                &|_| {}
            )
            .await,
        Err(crate::providers::llm::AgentTaskError::OutcomeUnknown)
    );
    assert!(account.stream_requests().is_empty());
    assert!(account.deleted_sessions().is_empty());
}

#[tokio::test]
async fn agent_task_gateway_collect_files_never_sends_an_attachment_reminder() {
    let account = FakeVmAccount::new(vec![FakeStreamTurn::chat_reply(
        vec![],
        "Saved [MUSE_TASK_DONE]",
    )]);
    let provider = build_gateway_muse_provider(
        &[SINGLE_COOKIE],
        FakeMuseVmTransport::new(vec![account.clone()]),
    )
    .await;
    let mut req = request(&format!("muse_task:{}", uuid::Uuid::new_v4()));
    provider
        .agent_task()
        .unwrap()
        .run_turn(&req, &Journal::default(), &|_| {})
        .await
        .unwrap();
    req.action = AgentTurnAction::Collect {
        output: AgentOutputKind::Files,
        filename: None,
    };
    tokio::time::pause();
    let result = provider
        .agent_task()
        .unwrap()
        .run_turn(&req, &Journal::default(), &|_| {})
        .await;
    assert_eq!(account.stream_requests().len(), 1);
    assert!(result.unwrap().files_missing);
}
