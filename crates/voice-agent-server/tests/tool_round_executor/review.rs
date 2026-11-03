use super::harness::*;

/// A guard on the harness itself: without a Device MCP peer that answers, the ordering and cap
/// tests above would pass for the wrong reason.
#[tokio::test]
async fn the_peer_really_is_a_device_mcp_server_and_the_external_catalog_really_resolves() {
    let external =
        ExternalServer::start(ExternalScript::with_tools(vec![external_tool("Forecast")])).await;
    let (llm, observed) = ScriptedLlm::new(vec![
        Round::ToolCalls(vec![("call-1", DEVICE_LLM_NAME, serde_json::json!({}))]),
        Round::Text("on"),
    ]);
    let url = database_url();
    let voice = start(config(url.clone(), LlmToolsConfig::default()), llm).await;
    seed_reviewed(&voice.base, &url, &external.url).await;

    let mut peer = Peer::open(&voice.base).await;
    peer.run_turn().await;
    peer.wait_for_turn_end().await;

    assert_eq!(peer.device_calls.len(), 1);
    let observed = observed
        .lock()
        .expect("the observation mailbox is not poisoned");
    let offered = &observed.offered[0];
    assert_eq!(
        offered
            .iter()
            .filter(|name| *name == DEVICE_LLM_NAME)
            .count(),
        1,
        "a Device MCP tool is advertised exactly once"
    );
    assert!(
        offered.contains(&EXTERNAL_LLM_NAME.to_owned()),
        "the External MCP catalog this session was admitted with is advertised too: {offered:?}"
    );
}

#[tokio::test]
async fn participating_agent_never_advertises_or_dispatches_unreviewed_external_tool() {
    let external =
        ExternalServer::start(ExternalScript::with_tools(vec![external_tool("Forecast")])).await;
    let (llm, observed) = ScriptedLlm::new(vec![
        Round::ToolCalls(vec![("call-1", EXTERNAL_LLM_NAME, serde_json::json!({}))]),
        Round::Text("unavailable"),
    ]);
    let url = database_url();
    let voice = start(config(url.clone(), LlmToolsConfig::default()), llm).await;
    seed(&url, &external.url).await;
    let pool = SqlitePoolOptions::new().connect(&url).await.unwrap();
    sqlx::query("INSERT INTO agent_speaker_policies(agent_id,mode) VALUES(1,'observe')")
        .execute(&pool)
        .await
        .unwrap();
    let mut peer = Peer::open(&voice.base).await;
    peer.run_turn().await;
    peer.wait_for_turn_end().await;
    assert!(external.script().calls().is_empty());
    let observed = observed.lock().unwrap();
    assert!(!observed.offered.is_empty());
    assert!(
        observed
            .offered
            .iter()
            .all(|names| !names.iter().any(|name| name == EXTERNAL_LLM_NAME))
    );
    voice.task.abort();
    external.task.abort();
}

#[tokio::test]
async fn reviewed_external_tool_is_advertised_and_dispatched_on_new_websocket() {
    let external =
        ExternalServer::start(ExternalScript::with_tools(vec![external_tool("Forecast")])).await;
    let (llm, observed) = ScriptedLlm::new(vec![
        Round::ToolCalls(vec![(
            "call-1",
            EXTERNAL_LLM_NAME,
            serde_json::json!({"city":"Hanoi"}),
        )]),
        Round::Text("forecast"),
    ]);
    let url = database_url();
    let voice = start(config(url.clone(), LlmToolsConfig::default()), llm).await;
    seed(&url, &external.url).await;
    let pool = SqlitePoolOptions::new().connect(&url).await.unwrap();
    sqlx::query("INSERT INTO agent_speaker_policies(agent_id,mode) VALUES(1,'observe')")
        .execute(&pool)
        .await
        .unwrap();
    approve_external_tool(&voice.base).await;
    let mut peer = Peer::open(&voice.base).await;
    peer.run_turn().await;
    peer.wait_for_turn_end().await;
    assert_eq!(external.script().calls(), vec!["Forecast"]);
    assert!(
        observed.lock().unwrap().offered[0]
            .iter()
            .any(|name| name == EXTERNAL_LLM_NAME)
    );
    voice.task.abort();
    external.task.abort();
}
