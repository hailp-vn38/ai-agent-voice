use super::*;

#[tokio::test]
async fn agent_task_gateway_lost_ack_reconciles_without_duplicate_post() {
    let account = FakeVmAccount::new(vec![FakeStreamTurn::chat_reply(
        vec![],
        "Recovered [MUSE_TASK_DONE]",
    )])
    .with_lost_stream_ack();
    let provider = build_gateway_muse_provider(
        &[SINGLE_COOKIE],
        FakeMuseVmTransport::new(vec![account.clone()]),
    )
    .await;
    let mut req = request(&format!("muse_task:{}", uuid::Uuid::new_v4()));
    let journal = Journal::default();
    assert_eq!(
        provider
            .agent_task()
            .unwrap()
            .run_turn(&req, &journal, &|_| {})
            .await,
        Err(crate::providers::llm::AgentTaskError::OutcomeUnknown)
    );
    replay(&mut req, &journal);
    let output = provider
        .agent_task()
        .unwrap()
        .run_turn(&req, &journal, &|_| {})
        .await
        .unwrap();
    assert_eq!(output.message, "Recovered");
    assert_eq!(account.stream_requests().len(), 1);
    assert!(account.deleted_sessions().is_empty());
}

#[tokio::test]
async fn agent_task_gateway_restore_keeps_largest_completed_turn_and_detects_lost_history() {
    use super::super::super::pool::restore_job_sessions_from_operations;
    let account = FakeVmAccount::new(vec![]);
    let provider = build_gateway_muse_provider(
        &[SINGLE_COOKIE],
        FakeMuseVmTransport::new(vec![account.clone()]),
    )
    .await;
    let mut req = request(&format!("muse_task:{}", uuid::Uuid::new_v4()));
    req.operation.account_key = Some(
        provider
            .hub
            .acquire_lease(&provider.config.gateway_binding)
            .unwrap()
            .account_key,
    );
    req.operation.remote_session_id = Some("restored-task".into());
    req.operation.batch_index = 2;
    let mut older = req.operation.clone();
    older.batch_index = 1;
    let operations = [req.operation.clone(), older].map(|operation| {
        orchestrator_agent_rs::ToolOperationSummary {
            operation,
            invocation_status: "completed".into(),
            root_session_id: "root".into(),
            root_has_active_work: true,
        }
    });
    restore_job_sessions_from_operations(&operations).await;
    account.set_history_polls(vec![vec![serde_json::json!({"role":"user"}), serde_json::json!({"role":"assistant","status":"completed","display_text":"old [MUSE_TASK_DONE]"})]]);
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
        Err(crate::providers::llm::AgentTaskError::SessionLost)
    );
    assert!(account.stream_requests().is_empty());
}

#[tokio::test]
async fn agent_task_gateway_reminder_recovers_after_journal_failure_without_resending() {
    let mut followup = FakeStreamTurn::chat_reply(vec![], "Still no file [MUSE_TASK_DONE]");
    followup.history_polls[0].insert(0, serde_json::json!({"role":"user"}));
    let account = FakeVmAccount::new(vec![
        FakeStreamTurn::chat_reply(vec![], "No file [MUSE_TASK_DONE]"),
        followup,
    ]);
    let provider = build_gateway_muse_provider(
        &[SINGLE_COOKIE],
        FakeMuseVmTransport::new(vec![account.clone()]),
    )
    .await;
    let mut req = request(&format!("muse_task:{}", uuid::Uuid::new_v4()));
    req.action = AgentTurnAction::Send {
        prompt: "Create report".into(),
        output: AgentOutputKind::Files,
        filename: None,
    };
    let journal = FailReminderAckJournal::default();
    assert_eq!(
        provider
            .agent_task()
            .unwrap()
            .run_turn(&req, &journal, &|_| {})
            .await,
        Err(crate::providers::llm::AgentTaskError::OutcomeUnknown)
    );
    replay(&mut req, &journal.saved);
    assert_eq!(req.operation.status, ToolOperationStatus::Submitting);
    let output = provider
        .agent_task()
        .unwrap()
        .run_turn(&req, &journal.saved, &|_| {})
        .await
        .unwrap();
    assert!(output.files_missing);
    assert_eq!(output.turn, 2);
    assert_eq!(account.stream_requests().len(), 2);
}

#[tokio::test]
async fn agent_task_gateway_running_and_downloading_never_post() {
    for status in [
        ToolOperationStatus::Running,
        ToolOperationStatus::Downloading,
    ] {
        let account = FakeVmAccount::new(vec![]);
        account.set_history_polls(vec![vec![serde_json::json!({"role":"user"}),serde_json::json!({"role":"assistant","status":"completed","display_text":"Saved [MUSE_TASK_DONE]"})]]);
        let provider = build_gateway_muse_provider(
            &[SINGLE_COOKIE],
            FakeMuseVmTransport::new(vec![account.clone()]),
        )
        .await;
        let mut req = request(&format!("muse_task:{}", uuid::Uuid::new_v4()));
        req.operation.status = status;
        req.operation.user_turns_before = Some(0);
        req.operation.remote_session_id = Some("retained".into());
        let output = provider
            .agent_task()
            .unwrap()
            .run_turn(&req, &Journal::default(), &|_| {})
            .await
            .unwrap();
        assert_eq!(output.message, "Saved");
        assert!(account.stream_requests().is_empty());
    }
}

#[tokio::test]
async fn agent_task_gateway_completed_session_404_is_lost_but_fresh_404_is_allowed() {
    let account = FakeVmAccount::new(vec![FakeStreamTurn::chat_reply(
        vec![],
        "First [MUSE_TASK_DONE]",
    )])
    .with_history_unavailable_before_first_turn();
    let provider = build_gateway_muse_provider(
        &[SINGLE_COOKIE],
        FakeMuseVmTransport::new(vec![account.clone()]),
    )
    .await;
    let req = request(&format!("muse_task:{}", uuid::Uuid::new_v4()));
    provider
        .agent_task()
        .unwrap()
        .run_turn(&req, &Journal::default(), &|_| {})
        .await
        .unwrap();
    account.set_history_failure_status(404);
    assert_eq!(
        provider
            .agent_task()
            .unwrap()
            .run_turn(&req, &Journal::default(), &|_| {})
            .await,
        Err(crate::providers::llm::AgentTaskError::SessionLost)
    );
    assert_eq!(account.stream_requests().len(), 1);
}

#[tokio::test]
async fn agent_task_gateway_downloading_uses_checkpoint_paths_when_history_omits_attachments() {
    let account = FakeVmAccount::new(vec![]).with_file("saved.csv", b"a,b".to_vec());
    account.set_history_polls(vec![vec![serde_json::json!({"role":"user"}),serde_json::json!({"role":"assistant","status":"completed","display_text":"Saved [MUSE_TASK_DONE]"})]]);
    let provider = build_gateway_muse_provider(
        &[SINGLE_COOKIE],
        FakeMuseVmTransport::new(vec![account.clone()]),
    )
    .await;
    let mut req = request(&format!("muse_task:{}", uuid::Uuid::new_v4()));
    req.operation.status = ToolOperationStatus::Downloading;
    req.operation.user_turns_before = Some(0);
    req.operation.remote_session_id = Some("retained".into());
    req.operation.batch_paths_json = Some("[\"saved.csv\"]".into());
    let output = provider
        .agent_task()
        .unwrap()
        .run_turn(&req, &Journal::default(), &|_| {})
        .await
        .unwrap();
    assert_eq!(output.files.len(), 1);
    assert_eq!(output.files[0].bytes, b"a,b");
    assert!(account.stream_requests().is_empty());
}

#[tokio::test]
async fn agent_task_gateway_submitting_before_post_replays_exact_pending_items() {
    let account = FakeVmAccount::new(vec![FakeStreamTurn::chat_reply(
        vec![],
        "Completed [MUSE_TASK_DONE]",
    )]);
    account.set_history_polls(vec![vec![]]);
    let provider = build_gateway_muse_provider(
        &[SINGLE_COOKIE],
        FakeMuseVmTransport::new(vec![account.clone()]),
    )
    .await;
    let mut req = request(&format!("muse_task:{}", uuid::Uuid::new_v4()));
    req.operation.remote_session_id = Some("retained".into());
    req.operation.user_turns_before = Some(0);
    req.operation.request_json=Some(serde_json::json!({"pending_turn_items":[{"type":"text","text":"exact checkpoint prompt"}]}).to_string());
    provider
        .agent_task()
        .unwrap()
        .run_turn(&req, &Journal::default(), &|_| {})
        .await
        .unwrap();
    let sent = account.stream_requests();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["items"][0]["text"], "exact checkpoint prompt");
    assert_eq!(sent[0]["session_id"], "retained");
}

#[tokio::test]
async fn agent_task_gateway_corrupt_submitting_journal_never_guesses_pending_prompt() {
    for saved in ["{broken", "{}", "{\"pending_turn_items\":\"bad\"}"] {
        let account = FakeVmAccount::new(vec![FakeStreamTurn::chat_reply(
            vec![],
            "Unexpected send [MUSE_TASK_DONE]",
        )]);
        account.set_history_polls(vec![vec![]]);
        let provider = build_gateway_muse_provider(
            &[SINGLE_COOKIE],
            FakeMuseVmTransport::new(vec![account.clone()]),
        )
        .await;
        let mut req = request(&format!("muse_task:{}", uuid::Uuid::new_v4()));
        req.operation.remote_session_id = Some("retained".into());
        req.operation.user_turns_before = Some(0);
        req.operation.request_json = Some(saved.into());
        assert_eq!(
            provider
                .agent_task()
                .unwrap()
                .run_turn(&req, &Journal::default(), &|_| {})
                .await,
            Err(crate::providers::llm::AgentTaskError::OutcomeUnknown)
        );
        assert!(account.stream_requests().is_empty());
    }
}

#[tokio::test]
async fn agent_task_gateway_http_500_after_post_remains_outcome_unknown() {
    let mut turn = FakeStreamTurn::chat_reply(vec![], "unused");
    turn.status = 500;
    let account = FakeVmAccount::new(vec![turn]);
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
                &Journal::default(),
                &|_| {}
            )
            .await,
        Err(crate::providers::llm::AgentTaskError::OutcomeUnknown)
    );
    assert_eq!(account.stream_requests().len(), 1);
    assert!(account.deleted_sessions().is_empty());
}
