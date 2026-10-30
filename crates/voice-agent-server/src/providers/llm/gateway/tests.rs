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
    providers::llm::{LlmEvent, LlmProvider, LlmRequest},
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

    let rate_limit_token = "Bearer tok_anthropic_code_1".to_string();
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
        account_concurrency: None,
        cdp_port: None,
        headless: None,
    };
    let provider = LlmGatewayProvider::with_hub(&cfg, Arc::clone(&hub)).expect("provider");

    // Call #1 hits acc1 -> 429 Too Many Requests -> marks acc1 in Cooldown AND automatically retries on acc2 within the same call
    let mut stream = provider
        .stream(LlmRequest::text_turn("hi".to_owned()))
        .await
        .expect("first call automatically fails over from acc1 (429) to acc2");
    let mut collected = String::new();
    while let Some(ev) = stream.next().await {
        match ev.expect("event") {
            LlmEvent::TextDelta(delta) => collected.push_str(&delta),
            LlmEvent::Finished => break,
            _ => {}
        }
    }
    assert_eq!(collected, "Xin chào từ Claude!");

    let views = hub
        .list_accounts(Some(GatewayVendor::Anthropic))
        .await
        .expect("list");
    let view1 = views
        .iter()
        .find(|a| a.account_key == acc1.account_key)
        .expect("view1");
    assert!(view1.cooldown_remaining_secs > 0);
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
            "/backend-api/codex/responses",
            post(|headers: HeaderMap, body: axum::body::Bytes| async move {
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
                let originator = headers
                    .get("originator")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("");
                assert_eq!(originator, "codex_cli_rs");

                let payload: serde_json::Value =
                    serde_json::from_slice(&body).expect("codex responses json body");
                assert_eq!(payload.get("store").and_then(|v| v.as_bool()), Some(false));
                assert_eq!(payload.get("stream").and_then(|v| v.as_bool()), Some(true));
                assert!(
                    payload
                        .get("instructions")
                        .and_then(|v| v.as_str())
                        .is_some_and(|s| !s.trim().is_empty()),
                    "Codex Responses API requires non-empty instructions"
                );
                assert!(
                    payload.get("input").and_then(|v| v.as_array()).is_some_and(|a| !a.is_empty()),
                    "Codex Responses API requires non-empty input array"
                );
                assert!(
                    payload.get("max_tokens").is_none()
                        && payload.get("max_output_tokens").is_none()
                        && payload.get("temperature").is_none(),
                    "Codex Responses API rejects max_tokens / max_output_tokens / temperature"
                );

                // Simulate ChatGPT Codex rejecting retired `gpt-5-codex` model so fallback to `gpt-5.6-terra` is verified
                let model = payload.get("model").and_then(|v| v.as_str()).unwrap_or("");
                if model == "gpt-5-codex" {
                    return (
                        StatusCode::BAD_REQUEST,
                        [("content-type", "application/json")],
                        r#"{"detail":"The 'gpt-5-codex' model is not supported when using Codex with a ChatGPT account.","error":{"type":"invalid_request_error","code":"model_not_found","message":"The 'gpt-5-codex' model is not supported when using Codex with a ChatGPT account."}}"#,
                    )
                        .into_response();
                }
                assert_eq!(model, "gpt-5.6-terra");

                let sse = concat!(
                    "event: response.created\n",
                    "data: {\"type\":\"response.created\",\"sequence_number\":0}\n\n",
                    "event: response.output_text.delta\n",
                    "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Codex diagnostic \"}\n\n",
                    "event: response.output_text.delta\n",
                    "data: {\"type\":\"response.output_text.delta\",\"delta\":\"& orchestrator OK\"}\n\n",
                    "event: response.completed\n",
                    "data: {\"type\":\"response.completed\",\"sequence_number\":3}\n\n"
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
    let codex_base_url = format!("http://{addr}/backend-api/codex");
    let claude_base_url = format!("http://{addr}/v1");

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
            "base_url": codex_base_url,
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
            "base_url": codex_base_url,
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
        base_url: codex_base_url.clone(),
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
        panic!(
            "expected ProviderAuth::CustomExecutor, got {:?}",
            orch_codex.auth
        );
    };

    let resolved_claude = ResolvedLlmProvider {
        provider_ref: "gw_claude".to_owned(),
        name: "Claude Pool".to_owned(),
        source: "database",
        adapter: "llm_gateway".to_owned(),
        model: "claude-sonnet-4-20250514".to_owned(),
        base_url: claude_base_url,
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

#[tokio::test]
async fn llm_gateway_provider_executes_muse_chat_vision_image_video_and_rotates_on_quota() {
    use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
    use serde_json::{Value, json};
    use std::collections::VecDeque;
    use std::sync::Mutex;
    use std::time::Duration;

    use crate::providers::llm::{
        LlmCapabilityTag, LlmOperation, LlmOutput, ReferenceImage,
        gateway::muse::{
            cdp::{BrowserEngine, BrowserPage, CdpError, CookieParam},
            config::NEW_THREAD_URL,
            dom::{
                CHAT_STATE_JS, CLICK_SEND_JS, DENY_APPROVALS_JS, INPUT_EMPTY_JS,
                LAST_BUBBLE_MEDIA_JS, PAGE_STATE_JS,
            },
        },
    };

    const PNG_BYTES: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F,
        0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00,
        0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49,
        0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];
    const MP4_BYTES: &[u8] = &[
        0x00, 0x00, 0x00, 0x18, b'f', b't', b'y', b'p', b'm', b'p', b'4', b'2', 0x00, 0x00, 0x00,
        0x00, b'm', b'p', b'4', b'2', b'i', b's', b'o', b'm',
    ];

    struct MockMusePage {
        url: Mutex<String>,
        states: Mutex<VecDeque<Value>>,
        last_state: Mutex<Value>,
    }

    impl MockMusePage {
        fn new(states: Vec<Value>) -> Arc<Self> {
            let last = states.last().cloned().unwrap_or_else(|| {
                json!({
                    "agentCount": 0,
                    "lastText": "",
                    "generating": false,
                    "approval": false,
                    "attachments": [],
                    "tail": ""
                })
            });
            Arc::new(Self {
                url: Mutex::new(NEW_THREAD_URL.to_owned()),
                states: Mutex::new(VecDeque::from(states)),
                last_state: Mutex::new(last),
            })
        }
    }

    #[async_trait::async_trait]
    impl BrowserPage for MockMusePage {
        async fn navigate(&self, url: &str) -> Result<(), CdpError> {
            *self.url.lock().unwrap() = url.to_owned();
            Ok(())
        }
        async fn current_url(&self) -> Result<String, CdpError> {
            Ok(self.url.lock().unwrap().clone())
        }
        async fn evaluate_json(
            &self,
            expression: &str,
            _await_promise: bool,
            _timeout: Duration,
        ) -> Result<Value, CdpError> {
            if expression == PAGE_STATE_JS {
                return Ok(json!({
                    "ready": true,
                    "hasInput": true,
                    "path": "/thread/new",
                    "head": "Muse Workspace"
                }));
            }
            if expression == CHAT_STATE_JS {
                let mut q = self.states.lock().unwrap();
                let st = if let Some(next) = q.pop_front() {
                    *self.last_state.lock().unwrap() = next.clone();
                    next
                } else {
                    self.last_state.lock().unwrap().clone()
                };
                return Ok(st);
            }
            if expression == DENY_APPROVALS_JS {
                return Ok(json!([]));
            }
            if expression == CLICK_SEND_JS {
                return Ok(json!("clicked"));
            }
            if expression == INPUT_EMPTY_JS {
                return Ok(json!(true));
            }
            if expression == LAST_BUBBLE_MEDIA_JS {
                return Ok(json!({ "links": [], "html": "" }));
            }
            if expression.contains("new File([bytes],")
                || expression.contains("HTMLTextAreaElement.prototype, 'value'")
            {
                return Ok(json!({ "ok": true, "len": 10 }));
            }
            if expression.contains("await fetch(") {
                if expression.contains("blob:https://muse.ai/img-1") {
                    return Ok(json!({
                        "ok": true,
                        "mime": "image/png",
                        "b64": BASE64.encode(PNG_BYTES),
                    }));
                }
                if expression.contains("blob:https://muse.ai/vid-1") {
                    return Ok(json!({
                        "ok": true,
                        "mime": "video/mp4",
                        "b64": BASE64.encode(MP4_BYTES),
                    }));
                }
            }
            Ok(Value::Null)
        }
        async fn press_enter(&self) -> Result<(), CdpError> {
            Ok(())
        }
        async fn close(&self) {}
    }

    struct MockMuseEngine {
        pages: Mutex<VecDeque<Arc<MockMusePage>>>,
    }

    #[async_trait::async_trait]
    impl BrowserEngine for MockMuseEngine {
        async fn open_page(
            &self,
            _context_key: &str,
            _cookies: &[CookieParam],
        ) -> Result<Arc<dyn BrowserPage>, CdpError> {
            let page = self
                .pages
                .lock()
                .unwrap()
                .pop_front()
                .expect("queued mock muse page");
            Ok(page)
        }
        async fn dispose_context(&self, _context_key: &str) {}
    }

    let hub = create_test_hub().await;
    // Register 2 Muse accounts in `pool:muse`
    let ch1 = hub
        .start_oauth_login(StartOAuthLoginRequest {
            vendor: GatewayVendor::Muse,
            label: "Muse #1".to_owned(),
            redirect_uri: None,
        })
        .await
        .expect("ch1");
    let acc1 = hub
        .complete_oauth_login(
            &ch1.session_id,
            OAuthCallbackSubmission {
                callback_url_or_code:
                    "hatch_sess=s1; hatch_gw=g1; hatch_vml=v1; hatch_native_auth_device=d1"
                        .to_owned(),
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
    .expect("priority 10");

    let ch2 = hub
        .start_oauth_login(StartOAuthLoginRequest {
            vendor: GatewayVendor::Muse,
            label: "Muse #2".to_owned(),
            redirect_uri: None,
        })
        .await
        .expect("ch2");
    let _acc2 = hub
        .complete_oauth_login(
            &ch2.session_id,
            OAuthCallbackSubmission {
                callback_url_or_code:
                    "hatch_sess=s2; hatch_gw=g2; hatch_vml=v2; hatch_native_auth_device=d2"
                        .to_owned(),
                state: None,
            },
        )
        .await
        .expect("acc2");

    // Page 1 (acc1): returns QuotaExceeded -> puts acc1 into cooldown in LlmGatewayHub
    let page_quota = MockMusePage::new(vec![
        json!({"agentCount": 0, "lastText": "", "generating": false, "approval": false, "attachments": [], "tail": ""}),
        json!({"agentCount": 1, "lastText": "You've hit your usage limit for this billing period.", "generating": false, "approval": false, "attachments": [], "tail": ""}),
    ]);
    // Page 2 (acc2): chat + image + video turns
    let page_acc2 = MockMusePage::new(vec![
        // Chat turn
        json!({"agentCount": 0, "lastText": "", "generating": false, "approval": false, "attachments": [], "tail": ""}),
        json!({"agentCount": 1, "lastText": "Xin chào từ Muse Gateway!", "generating": false, "approval": false, "attachments": [], "tail": ""}),
        json!({"agentCount": 1, "lastText": "Xin chào từ Muse Gateway!", "generating": false, "approval": false, "attachments": [], "tail": ""}),
        json!({"agentCount": 1, "lastText": "Xin chào từ Muse Gateway!", "generating": false, "approval": false, "attachments": [], "tail": ""}),
        json!({"agentCount": 1, "lastText": "Xin chào từ Muse Gateway!", "generating": false, "approval": false, "attachments": [], "tail": ""}),
        json!({"agentCount": 1, "lastText": "Xin chào từ Muse Gateway!", "generating": false, "approval": false, "attachments": [], "tail": ""}),
        // ImageGeneration turn
        json!({"agentCount": 1, "lastText": "Xin chào từ Muse Gateway!", "generating": false, "approval": false, "attachments": [], "tail": ""}),
        json!({"agentCount": 2, "lastText": "Here is the image", "generating": false, "approval": false, "attachments": [{"tid": "image-attachment", "kind": "image", "src": "blob:https://muse.ai/img-1", "w": 512, "h": 512}], "tail": ""}),
        // VideoGeneration turn
        json!({"agentCount": 2, "lastText": "Here is the image", "generating": false, "approval": false, "attachments": [], "tail": ""}),
        json!({"agentCount": 3, "lastText": "Here is the video [ALL_MEDIA_ATTACHED]", "generating": false, "approval": false, "attachments": [{"tid": "video-attachment", "kind": "video", "src": "blob:https://muse.ai/vid-1", "w": 1280, "h": 720}], "tail": ""}),
    ]);

    let engine: Arc<dyn BrowserEngine> = Arc::new(MockMuseEngine {
        pages: Mutex::new(VecDeque::from([page_quota, page_acc2])),
    });

    let cfg = LlmGatewayConfig {
        gateway_binding: GatewayBinding::Pool(GatewayVendor::Muse),
        model: "muse-chat".to_owned(),
        base_url: None,
        timeout_ms: 10_000,
        max_tokens: None,
        context_limit_tokens: None,
        reasoning_effort: None,
        temperature_percent: None,
        tags: None, // defaults to [chat, vision, image, video] for pool:muse
        account_concurrency: Some(2),
        cdp_port: Some(19222),
        headless: Some(true),
    };
    let provider = LlmGatewayProvider::with_hub_and_muse_engine(&cfg, Arc::clone(&hub), engine)
        .expect("provider");

    assert!(provider.supports_tag(LlmCapabilityTag::Chat));
    assert!(provider.supports_tag(LlmCapabilityTag::Vision));
    assert!(provider.supports_tag(LlmCapabilityTag::Image));
    assert!(provider.supports_tag(LlmCapabilityTag::Video));

    // 1. First Chat call hits acc1 -> QuotaExceeded -> marks acc1 in Cooldown AND automatically retries on acc2 within the same call
    let out_chat = provider
        .execute_operation(LlmOperation::Chat {
            request: LlmRequest::text_turn("hello".to_owned()),
            reasoning: false,
        })
        .await
        .expect("chat automatically fails over from acc1 (QuotaExceeded) to acc2");
    assert_eq!(
        out_chat,
        LlmOutput::Text("Xin chào từ Muse Gateway!".to_owned())
    );
    let views = hub.list_accounts(Some(GatewayVendor::Muse)).await.unwrap();
    let v1 = views
        .iter()
        .find(|a| a.account_key == acc1.account_key)
        .unwrap();
    assert!(
        v1.cooldown_remaining_secs > 0,
        "acc1 must be in cooldown after QuotaExceeded"
    );

    // 3. ImageGeneration via LlmGatewayProvider (`pool:muse`)
    let out_img = provider
        .execute_operation(LlmOperation::ImageGeneration {
            prompt: "a futuristic robot".to_owned(),
            width: 512,
            height: 512,
            reference_images: vec![ReferenceImage {
                bytes: PNG_BYTES.to_vec(),
                mime_type: "image/png".to_owned(),
            }],
            transparent_background: false,
        })
        .await
        .expect("image generation via acc2");
    assert_eq!(
        out_img,
        LlmOutput::single_media(PNG_BYTES.to_vec(), "image/png")
    );

    // 4. VideoGeneration via LlmGatewayProvider (`pool:muse`)
    let out_vid = provider
        .execute_operation(LlmOperation::VideoGeneration {
            prompt: "a drone flight".to_owned(),
            width: 1280,
            height: 720,
            duration_secs: 5,
            first_frame: None,
        })
        .await
        .expect("video generation via acc2");
    assert_eq!(
        out_vid,
        LlmOutput::single_media(MP4_BYTES.to_vec(), "video/mp4")
    );
}

#[tokio::test]
async fn llm_gateway_rejection_does_not_leave_provider_test_busy() {
    use std::collections::HashMap;
    use std::time::Duration;

    use crate::{
        providers::{
            DatabaseRuntimeSnapshot, DatabaseRuntimeState, DatabaseRuntimeStatus, ProviderType,
            RuntimeCatalog,
        },
        services::provider_diagnostic::{
            ProviderDiagnosticError, ProviderDiagnosticLimiter, ProviderDiagnosticService,
            ProviderDiagnosticTarget,
        },
        workers::LlmRuntime,
    };

    let hub = create_test_hub().await;
    let ch = hub
        .start_oauth_login(StartOAuthLoginRequest {
            vendor: GatewayVendor::Anthropic,
            label: "Claude Reject Test".to_owned(),
            redirect_uri: None,
        })
        .await
        .expect("start oauth");
    hub.complete_oauth_login(
        &ch.session_id,
        OAuthCallbackSubmission {
            callback_url_or_code: "code_reject_test".to_owned(),
            state: None,
        },
    )
    .await
    .expect("complete oauth");

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let server = tokio::spawn(async move {
        let _ = axum::serve(
            listener,
            Router::new().route("/v1/messages", post(|| async { StatusCode::BAD_REQUEST })),
        )
        .await;
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
        account_concurrency: None,
        cdp_port: None,
        headless: None,
    };
    let provider =
        Arc::new(LlmGatewayProvider::with_hub(&cfg, Arc::clone(&hub)).expect("provider"));

    // With concurrency = 2, only 1 slot is available for Diagnostic workload (total - reserved_voice = 1).
    let registry = Arc::new(RuntimeCatalog {
        llm: HashMap::from([(
            "gw-claude".into(),
            Arc::new(LlmRuntime::new(provider, 2, Duration::from_secs(1))),
        )]),
        ..RuntimeCatalog::default()
    });
    let snapshot = DatabaseRuntimeSnapshot::from_states([(
        "gw-claude".into(),
        DatabaseRuntimeState {
            provider_id: 14,
            desired_revision: 1,
            status: DatabaseRuntimeStatus::Loaded,
            failure: None,
        },
    )]);
    let service = ProviderDiagnosticService::new(
        Arc::clone(&registry),
        Some(Arc::new(snapshot)),
        None,
        ProviderDiagnosticLimiter::new(1),
        Duration::from_secs(1),
    );
    let target = ProviderDiagnosticTarget {
        key: "gw-claude".into(),
        provider_type: ProviderType::Llm,
        desired_revision: 1,
    };

    for _ in 0..2 {
        let operation = registry
            .llm_diagnostic("gw-claude", LlmRequest::text_turn("hello".into()), 1024)
            .expect("llm_diagnostic");
        assert_eq!(
            service
                .execute(target.clone(), operation, Duration::from_millis(500))
                .await
                .err(),
            Some(ProviderDiagnosticError::Failed)
        );
    }
    server.abort();
}

#[tokio::test]
async fn llm_gateway_gemini_antigravity_streams_tools_vision_and_auto_refreshes_on_401() {
    use serde_json::{Value, json};
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::{
        providers::llm::{ChatMessage, LlmOperation, LlmOutput, ToolDefinition},
        services::llm_gateway::GatewayAccountStatus,
    };

    let hub = create_test_hub().await;
    let ch = hub
        .start_oauth_login(StartOAuthLoginRequest {
            vendor: GatewayVendor::Gemini,
            label: "LamPhucHai03".to_owned(),
            redirect_uri: None,
        })
        .await
        .expect("start gemini oauth");
    let acc = hub
        .complete_oauth_login(
            &ch.session_id,
            OAuthCallbackSubmission {
                callback_url_or_code: "gemini_initial_code".to_owned(),
                state: None,
            },
        )
        .await
        .expect("complete gemini oauth");

    let stream_calls = Arc::new(AtomicUsize::new(0));
    let stream_calls_clone = Arc::clone(&stream_calls);

    let app = Router::new()
        .route(
            "/v1internal:streamGenerateContent",
            post(move |headers: HeaderMap, body: axum::body::Bytes| {
                let calls = Arc::clone(&stream_calls_clone);
                async move {
                    let call_no = calls.fetch_add(1, Ordering::SeqCst) + 1;
                    let auth = headers
                        .get("authorization")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("");
                    let ua = headers
                        .get("user-agent")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("");
                    assert_eq!(ua, "antigravity/hub/2.24.0 darwin/arm64");
                    assert!(
                        headers.get("x-goog-cloud-project").is_none(),
                        "internal project header must not be leaked to upstream"
                    );

                    // Simulate initial token expired (401) -> LlmGatewayProvider must auto-refresh via hub and retry!
                    if auth == "Bearer tok_gemini_gemini_initial_code" {
                        assert_eq!(call_no, 1);
                        return (StatusCode::UNAUTHORIZED, "token expired").into_response();
                    }
                    assert_eq!(auth, "Bearer tok_refreshed_gemini_1");

                    let payload: Value =
                        serde_json::from_slice(&body).expect("antigravity envelope json");
                    assert_eq!(
                        payload.get("project").and_then(Value::as_str),
                        Some("proj_gemini_test")
                    );
                    assert_eq!(
                        payload.get("model").and_then(Value::as_str),
                        Some("gemini-3.8-flash-medium(medium)")
                    );
                    assert_eq!(
                        payload.get("userAgent").and_then(Value::as_str),
                        Some("antigravity")
                    );
                    let req_obj = payload.get("request").expect("request object");
                    assert!(
                        req_obj
                            .get("sessionId")
                            .and_then(Value::as_str)
                            .is_some_and(|s| s.starts_with('-'))
                    );
                    assert!(req_obj.get("systemInstruction").is_some());
                    assert!(req_obj.get("tools").is_some());

                    let sse = concat!(
                        "data: {\"response\":{\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[{\"text\":\"internal thinking...\",\"thought\":true}]}}]}}\n\n",
                        "data: {\"response\":{\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[{\"text\":\"Xin chào từ \"}]}}]}}\n\n",
                        "data: {\"response\":{\"candidates\":[{\"content\":{\"role\":\"model\",\"parts\":[{\"text\":\"Gemini 3.8 Flash!\"},{\"functionCall\":{\"id\":\"call_order_1\",\"name\":\"lookup_order\",\"args\":{\"order_id\":\"ORD-99\"}}}]}}]}}\n\n"
                    );
                    (
                        StatusCode::OK,
                        [("content-type", "text/event-stream")],
                        sse,
                    )
                        .into_response()
                }
            }),
        )
        .route(
            "/v1internal:generateContent",
            post(|headers: HeaderMap, body: axum::body::Bytes| async move {
                let auth = headers
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("");
                assert_eq!(auth, "Bearer tok_refreshed_gemini_1");
                let payload: Value = serde_json::from_slice(&body).expect("vision json");
                assert_eq!(
                    payload.get("model").and_then(Value::as_str),
                    Some("gemini-3.8-flash-medium(medium)")
                );
                axum::Json(json!({
                    "response": {
                        "candidates": [
                            {
                                "content": {
                                    "role": "model",
                                    "parts": [
                                        { "text": "Ảnh hợp lệ từ Gemini Vision" }
                                    ]
                                }
                            }
                        ]
                    }
                }))
            }),
        );

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let cfg = LlmGatewayConfig {
        gateway_binding: GatewayBinding::Pool(GatewayVendor::Gemini),
        model: "Gemini-3.8-flash".to_owned(),
        base_url: Some(Url::parse(&format!("http://{addr}/v1internal")).expect("url")),
        timeout_ms: 5_000,
        max_tokens: Some(65_000),
        context_limit_tokens: Some(1_000_000),
        reasoning_effort: None,
        temperature_percent: Some(70),
        tags: Some(vec![
            "chat".to_owned(),
            "reasoning".to_owned(),
            "vision".to_owned(),
        ]),
        account_concurrency: None,
        cdp_port: None,
        headless: None,
    };
    let provider = LlmGatewayProvider::with_hub(&cfg, Arc::clone(&hub)).expect("provider");

    let mut stream = provider
        .stream(LlmRequest {
            messages: vec![
                ChatMessage::System {
                    content: "Bạn là trợ lý AI.".to_owned(),
                },
                ChatMessage::User {
                    content: "Kiểm tra đơn ORD-99".to_owned(),
                },
            ],
            tools: vec![ToolDefinition {
                name: "lookup_order".to_owned(),
                description: "Tra cứu đơn hàng".to_owned(),
                parameters: json!({"type": "object"}),
            }],
        })
        .await
        .expect("stream succeeds after reactive 401 token refresh");

    let mut text = String::new();
    let mut tool_calls = Vec::new();
    while let Some(ev) = stream.next().await {
        match ev.expect("stream event") {
            LlmEvent::TextDelta(delta) => text.push_str(&delta),
            LlmEvent::ToolCall(tc) => tool_calls.push(tc),
            LlmEvent::Finished => break,
        }
    }
    assert_eq!(text, "Xin chào từ Gemini 3.8 Flash!");
    assert_eq!(tool_calls.len(), 1);
    assert_eq!(tool_calls[0].id, "call_order_1");
    assert_eq!(tool_calls[0].name, "lookup_order");
    assert_eq!(tool_calls[0].arguments, json!({"order_id": "ORD-99"}));
    assert_eq!(stream_calls.load(Ordering::SeqCst), 2);

    // Account must remain Active (not ReauthRequired) after reactive token refresh succeeded
    let views = hub
        .list_accounts(Some(GatewayVendor::Gemini))
        .await
        .expect("list accounts");
    let view = views
        .iter()
        .find(|a| a.account_key == acc.account_key)
        .expect("gemini account");
    assert_eq!(view.status, GatewayAccountStatus::Active);

    // Verify Vision execution via v1internal:generateContent
    let vision_out = provider
        .execute_operation(LlmOperation::Vision {
            prompt: "Mô tả ảnh".to_owned(),
            image_bytes: vec![1, 2, 3, 4],
            mime_type: "image/png".to_owned(),
        })
        .await
        .expect("gemini vision");
    assert_eq!(
        vision_out,
        LlmOutput::Text("Ảnh hợp lệ từ Gemini Vision".to_owned())
    );
}

