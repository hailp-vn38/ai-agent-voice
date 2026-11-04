use super::*;

#[tokio::test]
async fn agent_task_gateway_reconnects_same_session_without_posting_again() {
    let account = FakeVmAccount::new(vec![FakeStreamTurn::chat_reply(
        vec![],
        "Recovered socket [MUSE_TASK_DONE]",
    )])
    .with_history_disconnect(false);
    let provider = build_gateway_muse_provider(
        &[SINGLE_COOKIE],
        FakeMuseVmTransport::new(vec![account.clone()]),
    )
    .await;
    let req = request(&format!("muse_task:{}", uuid::Uuid::new_v4()));
    let output = provider
        .agent_task()
        .unwrap()
        .run_turn(&req, &Journal::default(), &|_| {})
        .await
        .unwrap();
    assert_eq!(output.message, "Recovered socket");
    assert_eq!(account.stream_requests().len(), 1);
    assert!(account.connection_count() >= 2);
    assert!(account.deleted_sessions().is_empty());
}

#[tokio::test]
async fn agent_task_gateway_reconnect_gives_up_after_300_seconds() {
    let account = FakeVmAccount::new(vec![FakeStreamTurn::chat_reply(
        vec![],
        "Recovered socket [MUSE_TASK_DONE]",
    )])
    .with_history_disconnect(true);
    let provider = build_gateway_muse_provider(
        &[SINGLE_COOKIE],
        FakeMuseVmTransport::new(vec![account.clone()]),
    )
    .await;
    let mut req = request(&format!("muse_task:{}", uuid::Uuid::new_v4()));
    req.wait_slice = Duration::from_secs(1700);
    tokio::time::pause();
    let started = tokio::time::Instant::now();
    assert!(matches!(
        provider
            .agent_task()
            .unwrap()
            .run_turn(&req, &Journal::default(), &|_| {})
            .await,
        Err(crate::providers::llm::AgentTaskError::RetryLater(_))
    ));
    assert!(
        started.elapsed() >= Duration::from_secs(300)
            && started.elapsed() < Duration::from_secs(301)
    );
    assert_eq!(account.stream_requests().len(), 1);
    assert!(account.deleted_sessions().is_empty());
}

#[tokio::test]
async fn agent_task_gateway_pinned_auth_failure_never_selects_another_account() {
    let account = FakeVmAccount::new(vec![FakeStreamTurn::chat_reply(
        vec![],
        "First [MUSE_TASK_DONE]",
    )]);
    let other = FakeVmAccount::new(vec![]);
    let provider = build_gateway_muse_provider(
        &[SINGLE_COOKIE, super::super::harness::SECOND_COOKIE],
        FakeMuseVmTransport::new(vec![account.clone(), other.clone()]),
    )
    .await;
    let req = request(&format!("muse_task:{}", uuid::Uuid::new_v4()));
    let journal = Journal::default();
    provider
        .agent_task()
        .unwrap()
        .run_turn(&req, &journal, &|_| {})
        .await
        .unwrap();
    let mut saved = req.clone();
    replay(&mut saved, &journal);
    provider.hub.report_outcome(
        saved.operation.account_key.as_deref().unwrap(),
        crate::services::llm_gateway::GatewayCallOutcome::Unauthorized,
    );
    assert_eq!(
        provider
            .agent_task()
            .unwrap()
            .run_turn(&saved, &Journal::default(), &|_| {})
            .await,
        Err(crate::providers::llm::AgentTaskError::NeedsReauth)
    );
    assert!(other.stream_requests().is_empty());
    assert_eq!(account.stream_requests().len(), 1);
}

#[tokio::test]
async fn agent_task_gateway_transient_history_does_not_close_shared_socket() {
    let account = FakeVmAccount::new(vec![
        FakeStreamTurn::chat_reply(vec![], "First [MUSE_TASK_DONE]"),
        FakeStreamTurn::chat_reply(vec![], "Sibling [MUSE_TASK_DONE]"),
    ]);
    let provider = build_gateway_muse_provider(
        &[SINGLE_COOKIE],
        FakeMuseVmTransport::new(vec![account.clone()]),
    )
    .await;
    let mut req = request(&format!("muse_task:{}", uuid::Uuid::new_v4()));
    req.wait_slice = Duration::from_secs(3);
    account.set_history_failure_status(500);
    tokio::time::pause();
    assert!(matches!(
        provider
            .agent_task()
            .unwrap()
            .run_turn(&req, &Journal::default(), &|_| {})
            .await,
        Err(crate::providers::llm::AgentTaskError::RetryLater(_))
    ));
    tokio::time::resume();
    account.clear_history_failure();
    let sibling = request(&format!("muse_task:{}", uuid::Uuid::new_v4()));
    assert_eq!(
        provider
            .agent_task()
            .unwrap()
            .run_turn(&sibling, &Journal::default(), &|_| {})
            .await
            .unwrap()
            .message,
        "Sibling"
    );
    assert_eq!(account.connection_count(), 1);
    assert!(account.deleted_sessions().is_empty());
    let sent = account.stream_requests();
    assert_ne!(sent[0]["session_id"], sent[1]["session_id"]);
}

#[tokio::test]
async fn agent_task_gateway_quota_cooldown_collects_on_pinned_account() {
    let account = FakeVmAccount::new(vec![FakeStreamTurn::chat_reply(
        vec![],
        "Saved [MUSE_TASK_DONE]",
    )]);
    let other = FakeVmAccount::new(vec![]);
    let provider = build_gateway_muse_provider(
        &[SINGLE_COOKIE, super::super::harness::SECOND_COOKIE],
        FakeMuseVmTransport::new(vec![account.clone(), other.clone()]),
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
    replay(&mut req, &journal);
    provider.hub.report_outcome(
        req.operation.account_key.as_deref().unwrap(),
        crate::services::llm_gateway::GatewayCallOutcome::RateLimited {
            retry_after_secs: Some(3600),
        },
    );
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
        "Saved"
    );
    assert!(other.stream_requests().is_empty());
    assert_eq!(account.stream_requests().len(), 1);
}

#[tokio::test]
async fn agent_task_gateway_account_slot_wait_is_bounded_at_60_seconds() {
    let mut working = FakeStreamTurn::chat_reply(vec![], "working");
    working.history_polls[0][1]["status"] = serde_json::json!("streaming");
    let account = FakeVmAccount::new(vec![working.clone(), working]);
    let provider = std::sync::Arc::new(
        build_gateway_muse_provider(
            &[SINGLE_COOKIE],
            FakeMuseVmTransport::new(vec![account.clone()]),
        )
        .await,
    );
    tokio::time::pause();
    let mut tasks = Vec::new();
    for _ in 0..2 {
        let provider = provider.clone();
        tasks.push(tokio::spawn(async move {
            let mut req = request(&format!("muse_task:{}", uuid::Uuid::new_v4()));
            req.wait_slice = Duration::from_secs(1700);
            provider
                .agent_task()
                .unwrap()
                .run_turn(&req, &Journal::default(), &|_| {})
                .await
        }));
    }
    for _ in 0..1000 {
        if account.stream_requests().len() == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(account.stream_requests().len(), 2);
    let mut req = request(&format!("muse_task:{}", uuid::Uuid::new_v4()));
    req.wait_slice = Duration::from_secs(1700);
    let started = tokio::time::Instant::now();
    assert!(matches!(
        provider
            .agent_task()
            .unwrap()
            .run_turn(&req, &Journal::default(), &|_| {})
            .await,
        Err(crate::providers::llm::AgentTaskError::RetryLater(_))
    ));
    assert!(
        started.elapsed() >= Duration::from_secs(60) && started.elapsed() < Duration::from_secs(61)
    );
    assert_eq!(account.stream_requests().len(), 2);
    for task in tasks {
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
    }
}
