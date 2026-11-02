use super::*;

#[tokio::test]
async fn agent_task_gateway_returns_message_and_retains_session() {
    let account = FakeVmAccount::new(vec![FakeStreamTurn::chat_reply(
        vec![],
        "report [MUSE_TASK_DONE]",
    )]);
    let provider = build_gateway_muse_provider(
        &[SINGLE_COOKIE],
        FakeMuseVmTransport::new(vec![account.clone()]),
    )
    .await;
    let output = provider
        .agent_task()
        .expect("Muse supports agent task")
        .run_turn(
            &request(&format!("muse_task:{}", uuid::Uuid::new_v4())),
            &Journal::default(),
            &|_| {},
        )
        .await
        .unwrap();
    assert_eq!(output.message, "report");
    assert_eq!(output.turn, 1);
    assert!(account.deleted_sessions().is_empty());
}

#[tokio::test]
async fn agent_task_gateway_downloads_contract_markdown() {
    let key = format!("muse_task:{}", uuid::Uuid::new_v4());
    let path = format!("tasks/{}/report.md", key.trim_start_matches("muse_task:"));
    let account = FakeVmAccount::new(vec![FakeStreamTurn::chat_reply(
        vec![],
        "report [MUSE_TASK_DONE]",
    )])
    .with_file(&path, b"# Research".to_vec());
    let provider = build_gateway_muse_provider(
        &[SINGLE_COOKIE],
        FakeMuseVmTransport::new(vec![account.clone()]),
    )
    .await;
    let mut request = request(&key);
    request.action = AgentTurnAction::Send {
        prompt: "Research Rust".into(),
        output: AgentOutputKind::Markdown,
        filename: Some("report.md".into()),
    };
    let output = provider
        .agent_task()
        .unwrap()
        .run_turn(&request, &Journal::default(), &|_| {})
        .await
        .unwrap();
    assert_eq!(output.files.len(), 1);
    assert_eq!(output.files[0].bytes, b"# Research");
    assert_eq!(output.files[0].mime_type, "text/markdown");
    let prompt = account.stream_requests()[0]["items"][0]["text"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(prompt.contains(&path));
}

#[tokio::test]
async fn agent_task_gateway_files_reminds_exactly_once() {
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
    let output = provider
        .agent_task()
        .unwrap()
        .run_turn(&req, &Journal::default(), &|_| {})
        .await
        .unwrap();
    assert!(output.files_missing);
    assert_eq!(account.stream_requests().len(), 2);
}

#[tokio::test]
async fn agent_task_gateway_downloads_unique_supported_files_only() {
    let account=FakeVmAccount::new(vec![FakeStreamTurn::chat_reply(vec![],"See sandbox:///home/user/report.csv and sandbox:///home/user/report.csv plus sandbox:///home/user/tool.sh [MUSE_TASK_DONE]")]).with_file("report.csv",b"a,b\n1,2".to_vec()).with_file("tool.sh",b"echo nope".to_vec());
    let provider = build_gateway_muse_provider(
        &[SINGLE_COOKIE],
        FakeMuseVmTransport::new(vec![account.clone()]),
    )
    .await;
    let output = provider
        .agent_task()
        .unwrap()
        .run_turn(
            &request(&format!("muse_task:{}", uuid::Uuid::new_v4())),
            &Journal::default(),
            &|_| {},
        )
        .await
        .unwrap();
    assert_eq!(output.files.len(), 1);
    assert_eq!(output.files[0].filename, "report.csv");
    assert_eq!(output.files[0].mime_type, "text/csv");
}

#[tokio::test]
async fn agent_task_gateway_file_over_128_mib_returns_explicit_error() {
    let account = FakeVmAccount::new(vec![FakeStreamTurn::chat_reply(
        vec![],
        "sandbox:///home/user/large.pdf [MUSE_TASK_DONE]",
    )])
    .with_file("large.pdf", vec![0; 128 * 1024 * 1024 + 1]);
    let provider =
        build_gateway_muse_provider(&[SINGLE_COOKIE], FakeMuseVmTransport::new(vec![account]))
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
        Err(crate::providers::llm::AgentTaskError::Rejected(
            "Muse file exceeds artifact size limit".into()
        ))
    );
}

#[tokio::test]
async fn agent_task_gateway_denies_approval_even_when_history_looks_completed() {
    let mut turn = FakeStreamTurn::chat_reply(vec![], "Report [MUSE_TASK_DONE]");
    turn.events.insert(0,serde_json::json!({"event":"approval.requested","payload":{"approval_id":"task-approval","type":"browser"}}));
    let account = FakeVmAccount::new(vec![turn]);
    let provider = build_gateway_muse_provider(
        &[SINGLE_COOKIE],
        FakeMuseVmTransport::new(vec![account.clone()]),
    )
    .await;
    assert!(matches!(
        provider
            .agent_task()
            .unwrap()
            .run_turn(
                &request(&format!("muse_task:{}", uuid::Uuid::new_v4())),
                &Journal::default(),
                &|_| {}
            )
            .await,
        Err(crate::providers::llm::AgentTaskError::Rejected(_))
    ));
    assert_eq!(
        account.decided_approvals(),
        vec![("task-approval".into(), "deny".into())]
    );
}

#[tokio::test]
async fn agent_task_gateway_report_mentioning_rate_limit_is_not_provider_failure() {
    let message = "Research report: rate limit is a service policy. Clients should back off.";
    let account = FakeVmAccount::new(vec![FakeStreamTurn::chat_reply(
        vec![],
        &format!("{message} [MUSE_TASK_DONE]"),
    )]);
    let provider =
        build_gateway_muse_provider(&[SINGLE_COOKIE], FakeMuseVmTransport::new(vec![account]))
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
            .await
            .unwrap()
            .message,
        message
    );
}
