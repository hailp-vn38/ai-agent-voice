use std::time::Duration;

use super::harness::*;

/// Two sessions calling one server acquire from the process-global per-server bound, so one
/// session's call waits for a permit instead of a second call opening beside it.
#[tokio::test]
async fn two_sessions_calling_one_server_never_exceed_the_shared_per_server_bound() {
    let gate = CallGate::new();
    let external = ExternalServer::start(ExternalScript {
        gate: Some(gate.clone()),
        ..ExternalScript::with_tools(vec![external_tool("Forecast")])
    })
    .await;
    let (llm, _) = ScriptedLlm::new(vec![
        Round::ToolCalls(vec![("call-1", EXTERNAL_LLM_NAME, serde_json::json!({}))]),
        Round::ToolCalls(vec![("call-2", EXTERNAL_LLM_NAME, serde_json::json!({}))]),
    ]);
    let url = database_url();
    let voice = start(
        config_with_external(
            url.clone(),
            LlmToolsConfig {
                // Long enough that a call waiting for a permit is waited on rather than refused,
                // so what bounds the fan-out is the server's own concurrency and nothing else.
                execution_budget_ms: 30_000,
                ..Default::default()
            },
            ExternalMcpConfig {
                max_concurrent_calls_per_server: 1,
                ..ExternalMcpConfig::default()
            },
        ),
        llm,
    )
    .await;
    seed_reviewed(&voice.base, &url, &external.url).await;
    let script = external.script();

    let mut first = Peer::open(&voice.base).await;
    first.run_turn().await;
    gate.wait_until_held(1).await;

    let mut second = Peer::open_device(&voice.base, "second").await;
    second.run_turn().await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    assert_eq!(
        script.calls().len(),
        1,
        "the second session's call waits for the first one's permit rather than opening a second"
    );
    assert_eq!(
        script.peak_concurrency(),
        1,
        "the process-global per-server bound is what a call acquires from"
    );

    gate.open();
    // The gate stays open, so the second call runs as soon as the first releases its permit.
    gate.wait_until_held(2).await;
    first.pump(Duration::from_millis(300)).await;
    second.pump(Duration::from_millis(300)).await;
    assert_eq!(
        script.calls().len(),
        2,
        "the second call runs once the first has released its permit"
    );
    assert_eq!(
        script.peak_concurrency(),
        1,
        "the bound held for the whole turn, not only for the first call"
    );
}
