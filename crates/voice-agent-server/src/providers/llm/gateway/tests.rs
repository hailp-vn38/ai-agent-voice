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


