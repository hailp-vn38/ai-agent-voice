use super::*;
use crate::providers::llm::{ChatMessage, LlmEvent, LlmRequest};
use axum::response::sse::{Event, KeepAlive, Sse};
use futures_util::{StreamExt, stream};
use serde::Deserialize;
use std::{convert::Infallible, time::Duration};

const MAX_HISTORY_MESSAGES: usize = 12;
const MAX_HISTORY_BYTES: usize = 16 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AskInput {
    messages: Vec<AskMessage>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AskMessage {
    role: String,
    content: String,
}

pub(in crate::app::admin) async fn knowledge_ask(
    State(state): State<AppState>,
    request: Request,
) -> Response {
    let (request, input): (_, AskInput) = match json(request).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let messages = match validate_messages(input.messages) {
        Ok(messages) => messages,
        Err(code) => return error(&request, StatusCode::BAD_REQUEST, code),
    };

    let provider_ref = state
        .admin_task_control
        .lock()
        .expect("admin_task_control mutex poisoned")
        .background_settings
        .wiki_chat_llm_provider
        .clone();
    let Some(provider_ref) = provider_ref else {
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "wiki_chat_not_configured",
        );
    };
    let permit = match Arc::clone(&state.wiki_chat_admission).try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => return error(&request, StatusCode::TOO_MANY_REQUESTS, "wiki_chat_busy"),
    };

    let (provider, managed_lease) = if state.provider_runtime_manager.is_some() {
        match state
            .provider_diagnostics
            .managed_llm_provider(&provider_ref)
            .await
        {
            Ok((provider, lease)) => {
                if !provider.supports_tag(crate::providers::LlmCapabilityTag::Chat)
                    || !provider.supports_tools()
                {
                    return error(&request, StatusCode::SERVICE_UNAVAILABLE, "provider_failed");
                }
                (provider, Some(lease))
            }
            Err(_) => return error(&request, StatusCode::SERVICE_UNAVAILABLE, "provider_failed"),
        }
    } else {
        let key = provider_ref
            .strip_prefix("config:llm:")
            .unwrap_or(&provider_ref);
        let provider = match state.providers.llm(key) {
            Ok(provider) => provider,
            Err(_) => return error(&request, StatusCode::SERVICE_UNAVAILABLE, "provider_failed"),
        };
        if !provider.supports_tag(crate::providers::LlmCapabilityTag::Chat)
            || !provider.supports_tools()
        {
            return error(&request, StatusCode::SERVICE_UNAVAILABLE, "provider_failed");
        }
        (provider, None)
    };

    let provider_stream = match provider
        .stream(LlmRequest {
            messages,
            tools: Vec::new(),
        })
        .await
    {
        Ok(stream) => stream,
        Err(_) => {
            let error_event = Event::default()
                .event("error")
                .data(serde_json::json!({ "code": "provider_failed" }).to_string());
            let mut response = Sse::new(stream::iter([Ok::<_, Infallible>(error_event)]))
                .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
                .into_response();
            response.headers_mut().insert(
                axum::http::header::CACHE_CONTROL,
                axum::http::HeaderValue::from_static("no-store"),
            );
            return response;
        }
    };

    let events = stream::unfold(
        (provider_stream, Some(permit), managed_lease, false),
        |(mut provider_stream, permit, lease, done)| async move {
            if done {
                return None;
            }
            match provider_stream.next().await {
                Some(Ok(LlmEvent::TextDelta(text))) => {
                    let event = Event::default()
                        .event("delta")
                        .data(serde_json::json!({ "text": text }).to_string());
                    Some((
                        Ok::<_, Infallible>(event),
                        (provider_stream, permit, lease, false),
                    ))
                }
                Some(Ok(LlmEvent::Finished)) | None => {
                    let event = Event::default().event("done").data(
                        serde_json::json!({ "citations": [], "stop_reason": "finished" })
                            .to_string(),
                    );
                    Some((
                        Ok::<_, Infallible>(event),
                        (provider_stream, permit, lease, true),
                    ))
                }
                Some(Ok(LlmEvent::Activity)) => Some((
                    Ok::<_, Infallible>(Event::default().comment("working")),
                    (provider_stream, permit, lease, false),
                )),
                Some(Ok(LlmEvent::ToolCall(_))) | Some(Err(_)) => {
                    let event = Event::default()
                        .event("error")
                        .data(serde_json::json!({ "code": "provider_failed" }).to_string());
                    Some((
                        Ok::<_, Infallible>(event),
                        (provider_stream, permit, lease, true),
                    ))
                }
            }
        },
    );
    let mut response = Sse::new(events)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
        .into_response();
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response
}

fn validate_messages(messages: Vec<AskMessage>) -> Result<Vec<ChatMessage>, &'static str> {
    if messages.is_empty() || messages.len() > MAX_HISTORY_MESSAGES {
        return Err("invalid_input");
    }
    let total_bytes: usize = messages.iter().map(|message| message.content.len()).sum();
    if total_bytes > MAX_HISTORY_BYTES {
        return Err("invalid_input");
    }
    messages
        .into_iter()
        .map(|message| match message.role.as_str() {
            "user" => Ok(ChatMessage::User {
                content: message.content,
            }),
            "assistant" => Ok(ChatMessage::AssistantText {
                content: message.content,
            }),
            _ => Err("invalid_input"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        app::{AppState, router_with_state},
        config::AppConfig,
        database::DesiredProvider,
        lifecycle::AdmissionGate,
        providers::{
            LlmError, LlmEvent, LlmProvider, ProviderSet, RuntimeCatalog,
            llm::{LlmEventStream, LlmRequest},
        },
        services::provider_runtime::{
            ProviderRuntimeManager, RuntimeError, RuntimeLimits, RuntimeMaterializer,
            RuntimeResource,
        },
        workers::{LlmRuntime, ProviderRuntimeAdmission},
    };
    use futures_util::stream;
    use reqwest::StatusCode;
    use std::{sync::Arc, time::Duration};

    struct ScriptedLlm {
        supports_tools: bool,
    }

    struct ScriptedResource(Option<RuntimeCatalog>);
    impl RuntimeResource for ScriptedResource {
        fn unload(&self) -> bool {
            true
        }
        fn runtimes(&self) -> Option<RuntimeCatalog> {
            self.0.clone()
        }
    }

    struct ScriptedMaterializer(Arc<dyn LlmProvider>);
    impl RuntimeMaterializer for ScriptedMaterializer {
        fn estimated_peak_bytes(&self, _: &DesiredProvider) -> Result<u64, RuntimeError> {
            Ok(1)
        }
        fn logical_capacity(&self, _: &DesiredProvider) -> Result<usize, RuntimeError> {
            Ok(1)
        }
        fn build(
            &self,
            snapshot: &DesiredProvider,
            _: Option<crate::services::provider_runtime::PreparedRuntime>,
            _: ProviderRuntimeAdmission,
        ) -> Result<Arc<dyn RuntimeResource>, RuntimeError> {
            let mut runtimes = RuntimeCatalog::default();
            runtimes.llm.insert(
                snapshot.key.clone(),
                Arc::new(LlmRuntime::new(
                    Arc::clone(&self.0),
                    1,
                    Duration::from_secs(60),
                )),
            );
            Ok(Arc::new(ScriptedResource(Some(runtimes))))
        }
    }

    #[async_trait::async_trait]
    impl LlmProvider for ScriptedLlm {
        fn supports_tools(&self) -> bool {
            self.supports_tools
        }
        fn adapter(&self) -> &'static str {
            "test"
        }
        async fn stream(&self, request: LlmRequest) -> Result<LlmEventStream, LlmError> {
            assert!(request.tools.is_empty());
            Ok(Box::pin(stream::iter([
                Ok(LlmEvent::TextDelta("Wiki ".into())),
                Ok(LlmEvent::TextDelta("answer".into())),
                Ok(LlmEvent::Finished),
            ])))
        }
    }

    async fn fixture(configured: bool) -> (String, tokio::task::JoinHandle<()>) {
        fixture_with_tool_support(configured, true).await
    }

    async fn fixture_with_tool_support(
        configured: bool,
        supports_tools: bool,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let config: AppConfig = toml::from_str(
            r#"
[server]
bind = "127.0.0.1:0"
public_ws_url = "ws://127.0.0.1:0/voice/v1/"
[provider_defaults]
vad = "test"
asr = "test"
llm = "test"
tts = "test"
[api]
enabled = true
admin_token = "wiki-chat-test-token"
[providers.llm.instances.test]
adapter = "openai"
api_key = "test-key"
base_url = "https://api.openai.com/v1"
model = "fake"
"#,
        )
        .unwrap();
        let llm: Arc<dyn LlmProvider> = Arc::new(ScriptedLlm { supports_tools });
        let providers = Arc::new(ProviderSet::with_all(
            Arc::new(crate::providers::vad::UnavailableVad),
            Arc::new(crate::providers::asr::UnavailableAsr),
            llm,
            Arc::new(crate::providers::tts::UnavailableTts),
        ));
        let state = AppState::from_provider_set_with_database(config, providers, None);
        if configured {
            state
                .admin_task_control
                .lock()
                .unwrap()
                .background_settings
                .wiki_chat_llm_provider = Some("test".into());
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let router = router_with_state(state);
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        (format!("http://{address}/api/admin"), task)
    }

    async fn managed_fixture() -> (String, tokio::task::JoinHandle<()>) {
        let config: AppConfig = toml::from_str(
            r#"
            [server]
            bind = "127.0.0.1:0"
            public_ws_url = "ws://127.0.0.1:0/voice/v1/"
            [provider_defaults]
            vad = "test"
            asr = "test"
            llm = "managed_chat"
            tts = "test"
            [api]
            enabled = true
            admin_token = "wiki-chat-test-token"
            [providers.llm.instances.managed_chat]
            adapter = "openai"
            api_key = "test-key"
            base_url = "https://api.openai.com/v1"
            model = "fake"
            "#,
        )
        .unwrap();
        let fake: Arc<dyn LlmProvider> = Arc::new(ScriptedLlm {
            supports_tools: true,
        });
        let manager = ProviderRuntimeManager::new(
            RuntimeLimits {
                max_parallel_loads: 1,
                max_pending_loads: 0,
                max_waiters: 8,
                max_resident_bytes: 8,
                max_resources: 8,
                max_version_entries: 16,
                admission_timeout_ms: 1_000,
                failure_cooldown_ms: 10,
                idle_ttl_ms: 60_000,
            },
            Arc::new(ScriptedMaterializer(Arc::clone(&fake))),
            AdmissionGate::open(),
        )
        .unwrap();
        let providers = Arc::new(ProviderSet::with_all(
            Arc::new(crate::providers::vad::UnavailableVad),
            Arc::new(crate::providers::asr::UnavailableAsr),
            fake,
            Arc::new(crate::providers::tts::UnavailableTts),
        ));
        let state = AppState::from_provider_set_with_database(config, providers, None)
            .with_runtime_manager(manager);
        state
            .admin_task_control
            .lock()
            .unwrap()
            .background_settings
            .wiki_chat_llm_provider = Some("config:llm:managed_chat".into());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let router = router_with_state(state);
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        (format!("http://{address}/api/admin"), task)
    }

    #[tokio::test]
    async fn ask_streams_configured_provider_over_authenticated_http() {
        let (base, server) = fixture(true).await;
        let response = reqwest::Client::new()
            .post(format!("{base}/knowledge/ask"))
            .bearer_auth("wiki-chat-test-token")
            .json(&serde_json::json!({"messages":[{"role":"user","content":"question"}]}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["cache-control"], "no-store");
        assert!(
            response.headers()["content-type"]
                .to_str()
                .unwrap()
                .starts_with("text/event-stream")
        );
        let body = response.text().await.unwrap();
        assert!(body.contains("event: delta"));
        assert!(body.contains("Wiki "));
        assert!(body.contains("event: done"));
        server.abort();
    }

    #[tokio::test]
    async fn ask_streams_provider_from_managed_runtime_lease() {
        let (base, server) = managed_fixture().await;
        let response = reqwest::Client::new()
            .post(format!("{base}/knowledge/ask"))
            .bearer_auth("wiki-chat-test-token")
            .json(&serde_json::json!({"messages":[{"role":"user","content":"question"}]}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.text().await.unwrap();
        assert!(body.contains("event: delta"));
        assert!(body.contains("event: done"));
        server.abort();
    }

    #[tokio::test]
    async fn ask_without_provider_returns_not_configured() {
        let (base, server) = fixture(false).await;
        let response = reqwest::Client::new()
            .post(format!("{base}/knowledge/ask"))
            .bearer_auth("wiki-chat-test-token")
            .json(&serde_json::json!({"messages":[{"role":"user","content":"question"}]}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(
            response
                .text()
                .await
                .unwrap()
                .contains("wiki_chat_not_configured")
        );
        server.abort();
    }

    #[tokio::test]
    async fn chat_settings_reject_provider_without_tool_support() {
        let (base, server) = fixture_with_tool_support(false, false).await;
        let response = reqwest::Client::new()
            .put(format!("{base}/background/settings"))
            .bearer_auth("wiki-chat-test-token")
            .header("if-match", "\"1\"")
            .json(&serde_json::json!({
                "enabled": true,
                "wiki_chat_llm_provider": "config:llm:test"
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            response.json::<serde_json::Value>().await.unwrap()["error"]["code"],
            "provider_capability_mismatch"
        );
        server.abort();
    }

    #[test]
    fn chat_history_rejects_system_messages_and_byte_overflow() {
        assert!(
            validate_messages(vec![AskMessage {
                role: "system".into(),
                content: "x".into()
            }])
            .is_err()
        );
        assert!(
            validate_messages(vec![AskMessage {
                role: "user".into(),
                content: "x".repeat(MAX_HISTORY_BYTES + 1)
            }])
            .is_err()
        );
    }
}
