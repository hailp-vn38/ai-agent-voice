use std::{collections::BTreeMap, sync::Arc};

use axum::{
    Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::post,
};
use futures_util::StreamExt;
use tokio::net::TcpListener;
use url::Url;

use crate::{
    config::DatabaseConfig,
    database::{Database, credentials::CredentialCipher},
    providers::llm::{LlmError, LlmEvent, LlmProvider, LlmRequest},
    services::llm_gateway::{
        FakeOAuthVendorDriver, GatewayBinding, GatewayVendor, LlmGatewayHub,
        OAuthCallbackSubmission, OAuthVendorDriver, StartOAuthLoginRequest,
    },
};

use super::{LlmGatewayConfig, LlmGatewayProvider};

async fn create_test_hub() -> Arc<LlmGatewayHub> {
    let db = Database::connect(&DatabaseConfig {
        url: "sqlite::memory:".into(),
        max_connections: 1,
        ..Default::default()
    })
    .await
    .expect("sqlite memory");
    let cipher = Arc::new(CredentialCipher::new(1, &[0x42; 32]).expect("cipher"));
    let mut drivers: BTreeMap<GatewayVendor, Arc<dyn OAuthVendorDriver>> = BTreeMap::new();
    for vendor in GatewayVendor::ALL {
        drivers.insert(vendor, Arc::new(FakeOAuthVendorDriver::new(vendor)));
    }
    Arc::new(LlmGatewayHub::with_drivers(
        Some(db.pool().clone()),
        cipher,
        drivers,
    ))
}

#[tokio::test]
async fn llm_gateway_provider_streams_anthropic_and_rotates_on_429_without_retry() {
    let hub = create_test_hub().await;

    let ch1 = hub
        .start_oauth_login(StartOAuthLoginRequest {
            vendor: GatewayVendor::Anthropic,
            label: "Claude #1".to_owned(),
            redirect_uri: None,
        })
        .await
        .expect("ch1");
    let acc1 = hub
        .complete_oauth_login(
            &ch1.session_id,
            OAuthCallbackSubmission {
                callback_url_or_code: "code_1".to_owned(),
                state: None,
            },
        )
        .await
        .expect("acc1");
    hub.update_account(
        &acc1.account_key,
        crate::services::llm_gateway::UpdateGatewayAccountPatch {
            label: None,
            enabled: None,
            priority: Some(10),
        },
    )
    .await
    .expect("set acc1 priority 10 so it is deterministically first in pool");

    let ch2 = hub
        .start_oauth_login(StartOAuthLoginRequest {
            vendor: GatewayVendor::Anthropic,
            label: "Claude #2".to_owned(),
            redirect_uri: None,
        })
        .await
        .expect("ch2");
    let _acc2 = hub
        .complete_oauth_login(
            &ch2.session_id,
            OAuthCallbackSubmission {
                callback_url_or_code: "code_2".to_owned(),
                state: None,
            },
        )
        .await
        .expect("acc2");

    let rate_limit_token = format!("Bearer tok_anthropic_code_1");
    let app = Router::new()
        .route(
            "/v1/messages",
            post(
                |State(rl_tok): State<String>, headers: HeaderMap| async move {
                    let auth = headers
                        .get("authorization")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("");
                    let beta = headers
                        .get("anthropic-beta")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("");
                    assert_eq!(beta, "oauth-2025-04-20");

                    if auth == rl_tok {
                        return (
                            StatusCode::TOO_MANY_REQUESTS,
                            [("retry-after", "60")],
                            "rate limited",
                        )
                            .into_response();
                    }
                    let sse = concat!(
                        "event: content_block_delta\n",
                        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Xin chào \"}}\n\n",
                        "event: content_block_delta\n",
                        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"từ Claude!\"}}\n\n",
                        "event: message_stop\n",
                        "data: {\"type\":\"message_stop\"}\n\n"
                    );
                    (
                        StatusCode::OK,
                        [("content-type", "text/event-stream")],
                        sse,
                    )
                        .into_response()
                },
            ),
        )
        .with_state(rate_limit_token);

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let cfg = LlmGatewayConfig {
        gateway_binding: GatewayBinding::Pool(GatewayVendor::Anthropic),
        model: "claude-sonnet-4-20250514".to_owned(),
        base_url: Some(Url::parse(&format!("http://{addr}/v1")).expect("url")),
        timeout_ms: 5_000,
        max_tokens: Some(512),
        context_limit_tokens: None,
        reasoning_effort: None,
        temperature_percent: None,
        tags: Some(vec!["chat".to_owned()]),
    };
    let provider = LlmGatewayProvider::with_hub(&cfg, Arc::clone(&hub)).expect("provider");

    // Call #1 hits acc1 -> 429 Too Many Requests -> returns Err(LlmError::Failed) immediately (0 retry per ADR-0023)
    // and marks acc1 in Cooldown.
    let err = match provider
        .stream(LlmRequest::text_turn("hi".to_owned()))
        .await
    {
        Ok(_) => panic!("expected first call on acc1 to fail with 429"),
        Err(e) => e,
    };
    assert_eq!(err, LlmError::Failed);
    let views = hub
        .list_accounts(Some(GatewayVendor::Anthropic))
        .await
        .expect("list");
    let view1 = views
        .iter()
        .find(|a| a.account_key == acc1.account_key)
        .expect("view1");
    assert!(view1.cooldown_remaining_secs > 0);

    // Call #2 automatically skips cooled-down acc1 and streams from acc2
    let mut stream = provider
        .stream(LlmRequest::text_turn("hi again".to_owned()))
        .await
        .expect("second call succeeds via acc2");
    let mut collected = String::new();
    while let Some(ev) = stream.next().await {
        match ev.expect("event") {
            LlmEvent::TextDelta(delta) => collected.push_str(&delta),
            LlmEvent::Finished => break,
            _ => {}
        }
    }
    assert_eq!(collected, "Xin chào từ Claude!");
}

#[tokio::test]
async fn openai_adapter_with_gateway_binding_delegates_to_llm_gateway_provider_and_validates_raw() {
    let validated_gw = crate::database::providers::config::validate_raw(
        "llm_gateway",
        r#"{"gateway_binding":"pool:openai_codex","model":"gpt-5.3-codex"}"#,
    )
    .expect("llm_gateway config should validate");
    assert!(validated_gw.contains("pool:openai_codex"));

    let validated_openai = crate::database::providers::config::validate_raw(
        "openai",
        r#"{"model":"gpt-5.3-codex","gateway_binding":"pool:openai_codex"}"#,
    )
    .expect("openai config with gateway_binding should validate without api_key or base_url");
    assert!(validated_openai.contains("gateway_binding"));

    let openai_cfg = crate::config::OpenAiConfig {
        base_url: Url::parse("https://api.openai.com/v1").expect("url"),
        api_key: crate::config::SecretString::default(),
        gateway_binding: Some(GatewayBinding::Pool(GatewayVendor::OpenaiCodex)),
        model: "gpt-5.3-codex".to_owned(),
        timeout_ms: 5_000,
        max_tokens: Some(512),
        context_limit_tokens: None,
        reasoning_effort: None,
        temperature_percent: None,
        tags: Some(vec!["chat".to_owned()]),
    };
    let instance = crate::config::LlmInstanceConfig::Openai(openai_cfg);
    let factory = crate::providers::compiled_provider_registry()
        .llm_factory("openai")
        .expect("openai factory");
    let provider = factory
        .build(&instance)
        .expect("openai factory delegates to LlmGatewayProvider when gateway_binding is set");
    assert_eq!(provider.adapter(), "openai");

    // Verify `provider_secret_env_for_config` suppresses env fallback for gateway-bound openai rows
    assert!(
        crate::database::secrets::provider_secret_env_for_config(
            "llm_gw_openai",
            "openai",
            &validated_openai
        )
        .is_none()
    );
    assert_eq!(
        crate::database::secrets::provider_secret_env_for_config(
            "llm_plain_openai",
            "openai",
            r#"{"base_url":"https://api.openai.com/v1","model":"gpt-4o"}"#
        )
        .as_deref(),
        Some("VOICE_PROVIDER_LLM_PLAIN_OPENAI_API_KEY")
    );
}

#[tokio::test]
async fn llm_gateway_provider_diagnostic_and_orchestrator_custom_executor() {
    use std::collections::HashMap;
    use tokio_util::sync::CancellationToken;

    use crate::{
        app::{AppState, new_lifecycle_from_grace_ms},
        config::AppConfig,
        database::DesiredProvider,
        providers::LoadedProviders,
        services::{
            llm_gateway::SharedGatewayHub,
            provider_runtime::{FactoryMaterializer, ProviderRuntimeManager, RuntimeLimits},
            provider_test::{
                ProviderTestDraft, ProviderTestInput, ProviderTestOutput, ProviderTestRunner,
            },
            task_flow::ResolvedLlmProvider,
        },
    };
    use orchestrator_agent_rs::{
        ModelMessage, ModelParams, ModelRequest, ModelRole, ModelTool, ProviderAuth, StreamEvent,
    };

    let hub = create_test_hub().await;
    SharedGatewayHub::set_global(Arc::clone(&hub));

    // 1. Login OpenaiCodex account (FakeOAuthVendorDriver attaches `ChatGPT-Account-Id: acct_fake_codex`)
    let ch_codex = hub
        .start_oauth_login(StartOAuthLoginRequest {
            vendor: GatewayVendor::OpenaiCodex,
            label: "Codex Team #1".to_owned(),
            redirect_uri: None,
        })
        .await
        .expect("start codex");
    hub.complete_oauth_login(
        &ch_codex.session_id,
        OAuthCallbackSubmission {
            callback_url_or_code: "codex_code_1".to_owned(),
            state: None,
        },
    )
    .await
    .expect("complete codex");

    // 2. Login Anthropic account (FakeOAuthVendorDriver attaches `anthropic-beta: oauth-2025-04-20`)
    let ch_claude = hub
        .start_oauth_login(StartOAuthLoginRequest {
            vendor: GatewayVendor::Anthropic,
            label: "Claude Team #1".to_owned(),
            redirect_uri: None,
        })
        .await
        .expect("start claude");
    hub.complete_oauth_login(
        &ch_claude.session_id,
        OAuthCallbackSubmission {
            callback_url_or_code: "claude_code_1".to_owned(),
            state: None,
        },
    )
    .await
    .expect("complete claude");

    let app = Router::new()
        .route(
            "/v1/chat/completions",
            post(|headers: HeaderMap| async move {
                let auth = headers
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("");
                assert_eq!(auth, "Bearer tok_openai_codex_codex_code_1");
                let acct_id = headers
                    .get("chatgpt-account-id")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("");
                assert_eq!(acct_id, "acct_codex_test");

                let sse = concat!(
                    "data: {\"choices\":[{\"delta\":{\"content\":\"Codex diagnostic \"}}]}\n\n",
                    "data: {\"choices\":[{\"delta\":{\"content\":\"& orchestrator OK\"}}]}\n\n",
                    "data: [DONE]\n\n"
                );
                (
                    StatusCode::OK,
                    [("content-type", "text/event-stream")],
                    sse,
                )
                    .into_response()
            }),
        )
        .route(
            "/v1/messages",
            post(|headers: HeaderMap| async move {
                let auth = headers
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("");
                assert_eq!(auth, "Bearer tok_anthropic_claude_code_1");
                let beta = headers
                    .get("anthropic-beta")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("");
                assert_eq!(beta, "oauth-2025-04-20");

                let sse = concat!(
                    "event: content_block_delta\n",
                    "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Đang tra cứu Wiki...\"}}\n\n",
                    "event: content_block_start\n",
                    "data: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_01\",\"name\":\"wiki_search\",\"input\":{}}}\n\n",
                    "event: content_block_delta\n",
                    "data: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"query\\\":\\\"Rust\\\"}\"}}\n\n",
                    "event: content_block_stop\n",
                    "data: {\"type\":\"content_block_stop\",\"index\":1}\n\n",
                    "event: message_stop\n",
                    "data: {\"type\":\"message_stop\"}\n\n"
                );
                (
                    StatusCode::OK,
                    [("content-type", "text/event-stream")],
                    sse,
                )
                    .into_response()
            }),
        );

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let base_url = format!("http://{addr}/v1");

    // 3. Verify `ProviderTestRunner::run` with `adapter = "llm_gateway"` and `gateway_binding = "pool:openai_codex"`
    let cfg_path =
        std::env::temp_dir().join(format!("llm-gw-draft-test-{}.toml", uuid::Uuid::new_v4()));
    std::fs::write(
        &cfg_path,
        format!(
            r#"
[server]
bind = "127.0.0.1:0"
public_ws_url = "ws://127.0.0.1:0/voice/v1/"
[provider_defaults]
vad = "test"
asr = "test"
llm = "test"
tts = "test"
[database]
url = "sqlite://{}"
[api]
enabled = true
admin_token = "gw-draft-test-token"
"#,
            cfg_path.with_extension("db").display()
        ),
    )
    .expect("write test config");
    let app_config = AppConfig::parse_and_resolve(&cfg_path).expect("parse config");
    let _ = std::fs::remove_file(&cfg_path);
    let cipher: Arc<dyn crate::database::secrets::SecretResolver> =
        Arc::new(CredentialCipher::new(1, &[0x42; 32]).expect("cipher"));
    let lifecycle = new_lifecycle_from_grace_ms(1_000);
    let mut state = AppState::new_with_database_runtime_snapshot_resolver_and_shutdown(
        app_config.clone(),
        LoadedProviders::default(),
        None,
        None,
        Arc::clone(&cipher),
        lifecycle.clone(),
    );
    let builder = FactoryMaterializer::new(
        Arc::new(app_config),
        Arc::clone(&cipher),
        HashMap::from([("llm_gateway".into(), 1), ("openai".into(), 1)]),
        state.worker_supervisor.clone(),
    )
    .expect("factory materializer");
    let manager = ProviderRuntimeManager::new(
        RuntimeLimits {
            max_parallel_loads: 1,
            max_pending_loads: 2,
            max_waiters: 8,
            max_resident_bytes: 1024 * 1024,
            max_resources: 8,
            max_version_entries: 16,
            admission_timeout_ms: 5_000,
            failure_cooldown_ms: 1,
            idle_ttl_ms: 1,
        },
        Arc::new(builder),
        lifecycle.gate().clone(),
    )
    .expect("runtime manager");
    state = state.with_runtime_manager(manager);

    let runner = ProviderTestRunner {
        diagnostics: &state.provider_diagnostics,
        database: None,
        secrets: Arc::clone(&cipher),
    };
    let draft = ProviderTestDraft {
        kind: "llm".to_owned(),
        adapter: "llm_gateway".to_owned(),
        config_json: serde_json::json!({
            "gateway_binding": "pool:openai_codex",
            "model": "gpt-5-codex",
            "base_url": base_url,
        }),
        api_key: None,
        saved_credential: None,
    };
    let diag_out = runner
        .run(draft, ProviderTestInput::Llm("ping".to_owned()))
        .await
        .expect("ProviderTestRunner::run on llm_gateway should succeed");
    match diag_out {
        ProviderTestOutput::Text { text, .. } => {
            assert_eq!(text, "Codex diagnostic & orchestrator OK");
        }
        _ => panic!("expected text output from LLM diagnostic"),
    }

    // Also verify `DesiredProvider` materialization with `adapter = "llm_gateway"` via `ProviderRuntimeManager`
    let desired = DesiredProvider {
        id: 1,
        key: "gw_codex_db".to_owned(),
        kind: "llm".to_owned(),
        adapter: "llm_gateway".to_owned(),
        config_json: serde_json::json!({
            "gateway_binding": "pool:openai_codex",
            "model": "gpt-5-codex",
            "base_url": base_url,
        })
        .to_string(),
        secret_ref: None,
        revision: 1,
    };
    let rt_manager = state
        .provider_runtime_manager
        .as_ref()
        .expect("runtime manager");
    let lease = rt_manager
        .acquire_until(desired, rt_manager.admission_deadline())
        .await
        .expect("DesiredProvider with adapter=llm_gateway should materialize");
    assert_eq!(
        lease
            .runtimes()
            .expect("runtimes")
            .llm
            .get("gw_codex_db")
            .expect("llm instance")
            .provider()
            .adapter(),
        "llm_gateway"
    );

    // 4. Verify `ResolvedLlmProvider::to_orchestrator_provider()` produces `ProviderAuth::CustomExecutor`
    // for both OpenAI Codex and Anthropic, preserving `extra_headers` and tool calls
    let resolved_codex = ResolvedLlmProvider {
        provider_ref: "gw_codex".to_owned(),
        name: "Codex Pool".to_owned(),
        source: "database",
        adapter: "llm_gateway".to_owned(),
        model: "gpt-5-codex".to_owned(),
        base_url: base_url.clone(),
        api_key: None,
        gateway_binding: Some(GatewayBinding::Pool(GatewayVendor::OpenaiCodex)),
        timeout_ms: 5_000,
        max_tokens: 1024,
        context_limit_tokens: 128_000,
        reasoning_effort: None,
        temperature_percent: None,
        tags: vec!["chat".to_owned()],
        muse_options: None,
    };
    let orch_codex = resolved_codex.to_orchestrator_provider();
    let ProviderAuth::CustomExecutor(codex_exec) = &orch_codex.auth else {
        panic!("expected ProviderAuth::CustomExecutor, got {:?}", orch_codex.auth);
    };

    let resolved_claude = ResolvedLlmProvider {
        provider_ref: "gw_claude".to_owned(),
        name: "Claude Pool".to_owned(),
        source: "database",
        adapter: "llm_gateway".to_owned(),
        model: "claude-sonnet-4-20250514".to_owned(),
        base_url,
        api_key: None,
        gateway_binding: Some(GatewayBinding::Pool(GatewayVendor::Anthropic)),
        timeout_ms: 5_000,
        max_tokens: 1024,
        context_limit_tokens: 128_000,
        reasoning_effort: None,
        temperature_percent: None,
        tags: vec!["chat".to_owned()],
        muse_options: None,
    };
    let orch_claude = resolved_claude.to_orchestrator_provider();
    let ProviderAuth::CustomExecutor(claude_exec) = &orch_claude.auth else {
        panic!("expected ProviderAuth::CustomExecutor for claude");
    };

    let codex_req = ModelRequest {
        provider_id: "gw_codex".to_owned(),
        model_id: "gpt-5-codex".to_owned(),
        system: "You are a helpful coding assistant.".to_owned(),
        messages: vec![ModelMessage::text(ModelRole::User, "check status")],
        tools: Vec::new(),
        params: ModelParams::default(),
    };
    let codex_resp = codex_exec
        .complete(&codex_req, &CancellationToken::new(), &|_| {})
        .await
        .expect("codex custom executor complete");
    assert_eq!(codex_resp.text, "Codex diagnostic & orchestrator OK");

    let claude_events = std::sync::Mutex::new(Vec::new());
    let sink = |ev: StreamEvent| claude_events.lock().unwrap().push(ev);
    let claude_req = ModelRequest {
        provider_id: "gw_claude".to_owned(),
        model_id: "claude-sonnet-4-20250514".to_owned(),
        system: "You are Wiki Root.".to_owned(),
        messages: vec![ModelMessage::text(ModelRole::User, "Search wiki for Rust")],
        tools: vec![ModelTool {
            name: "wiki_search".to_owned(),
            description: "Search wiki".to_owned(),
            input_schema: serde_json::json!({"type":"object"}),
        }],
        params: ModelParams {
            max_output_tokens: Some(512),
            ..Default::default()
        },
    };
    let claude_resp = claude_exec
        .complete(&claude_req, &CancellationToken::new(), &sink)
        .await
        .expect("claude custom executor complete");
    assert_eq!(claude_resp.text, "Đang tra cứu Wiki...");
    assert_eq!(claude_resp.tool_calls.len(), 1);
    assert_eq!(claude_resp.tool_calls[0].id, "toolu_01");
    assert_eq!(claude_resp.tool_calls[0].name, "wiki_search");
    assert_eq!(
        claude_resp.tool_calls[0].arguments,
        serde_json::json!({"query": "Rust"})
    );
    let seen_events = claude_events.into_inner().unwrap();
    assert!(seen_events.contains(&StreamEvent::TextDelta("Đang tra cứu Wiki...".to_owned())));
    assert!(seen_events.contains(&StreamEvent::ToolCallReady {
        id: "toolu_01".to_owned(),
        name: "wiki_search".to_owned(),
    }));
}


