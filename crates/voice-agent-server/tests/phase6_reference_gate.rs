use std::{sync::Arc, time::Duration};

use futures_util::stream;
use tokio::{net::TcpListener, task::JoinHandle};
use url::Url;
use voice_agent_server::{
    app::router_with_providers,
    audio::PcmF32Mono,
    config::{
        AppConfig, AudioConfig, AuthConfig, BargeInConfig, DeploymentConfig, LimitsConfig,
        LlmConfig, McpConfig, ProvidersConfig, RuntimeConfig, ServerConfig, SpeechOutputConfig,
        TtsConfig, WebsocketConfig, WorkersConfig,
    },
    providers::llm::{ChatMessage, LlmEventStream, LlmRequest, ToolCall},
    providers::{
        AsrError, AsrEvent, AsrProvider, AsrResult, AsrSession, LlmError, LlmEvent, LlmProvider,
        ProviderSet, TtsError, TtsProvider, VadError, VadInput, VadProbability, VadProvider,
        VadSession,
    },
};

struct FakeVad;
impl VadProvider for FakeVad {
    fn adapter(&self) -> &'static str {
        "fake"
    }
    fn open(&self) -> Result<Box<dyn VadSession>, VadError> {
        Ok(Box::new(FakeVadSession))
    }
}
struct FakeVadSession;
impl VadSession for FakeVadSession {
    fn push(&mut self, input: VadInput) -> Result<VadProbability, VadError> {
        Ok(VadProbability {
            start_sample: input.start_sample,
            end_sample: input.start_sample + input.pcm.len() as u64,
            probability: 0.0,
        })
    }
    fn reset(&mut self) -> Result<(), VadError> {
        Ok(())
    }
}
struct FakeAsr;
impl AsrProvider for FakeAsr {
    fn open(&self) -> Result<Box<dyn AsrSession>, AsrError> {
        Ok(Box::new(FakeAsrSession))
    }
}
struct FakeAsrSession;
impl AsrSession for FakeAsrSession {
    fn push_pcm(&mut self, _: &PcmF32Mono) -> Result<Vec<AsrEvent>, AsrError> {
        Ok(Vec::new())
    }
    fn finish(&mut self) -> Result<AsrResult, AsrError> {
        Ok(AsrResult::new("unused"))
    }
    fn cancel(&mut self) {}
}
struct ScriptedLlm;
#[async_trait::async_trait]
impl LlmProvider for ScriptedLlm {
    fn adapter(&self) -> &'static str {
        "scripted"
    }
    async fn stream(&self, request: LlmRequest) -> Result<LlmEventStream, LlmError> {
        let has_result = request
            .messages
            .iter()
            .any(|message| matches!(message, ChatMessage::ToolResult { .. }));
        let user_text = request.messages.iter().find_map(|message| match message {
            ChatMessage::User { content } => Some(content.as_str()),
            _ => None,
        });
        let events = if has_result && user_text == Some("Giá trị hiện tại là bao nhiêu?") {
            vec![
                Ok(LlmEvent::TextDelta("Giá trị hiện tại là 50.".into())),
                Ok(LlmEvent::Finished),
            ]
        } else if has_result {
            vec![
                Ok(LlmEvent::TextDelta("Đã đặt giá trị thành 50.".into())),
                Ok(LlmEvent::Finished),
            ]
        } else if user_text == Some("Giá trị hiện tại là bao nhiêu?") {
            vec![
                Ok(LlmEvent::ToolCall(ToolCall {
                    id: "call-2".into(),
                    name: "test_get_value".into(),
                    arguments: serde_json::json!({}),
                })),
                Ok(LlmEvent::Finished),
            ]
        } else {
            vec![
                Ok(LlmEvent::TextDelta("Tôi sẽ kiểm tra.".into())),
                Ok(LlmEvent::ToolCall(ToolCall {
                    id: "call-1".into(),
                    name: "test_set_value".into(),
                    arguments: serde_json::json!({"value":50}),
                })),
                Ok(LlmEvent::Finished),
            ]
        };
        Ok(Box::pin(stream::iter(events)))
    }
}
struct FakeTts;
impl TtsProvider for FakeTts {
    fn adapter(&self) -> &'static str {
        "fake"
    }
    fn synthesize(&self, _: &str) -> Result<PcmF32Mono, TtsError> {
        Ok(PcmF32Mono::new(vec![0.1; 2_880], 48_000))
    }
}

async fn start() -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let config = AppConfig {
        server: ServerConfig {
            bind: address,
            public_ws_url: Url::parse(&format!("ws://{address}/voice/v1/")).unwrap(),
            hello_timeout_ms: 500,
        },
        auth: AuthConfig::default(),
        audio: AudioConfig::default(),
        websocket: WebsocketConfig::default(),
        limits: LimitsConfig::default(),
        providers: ProvidersConfig::default(),
        workers: WorkersConfig::default(),
        deployment: DeploymentConfig::default(),
        runtime: RuntimeConfig::default(),
        llm: LlmConfig::default(),
        tts: TtsConfig::default(),
        speech_output: SpeechOutputConfig::default(),
        barge_in: BargeInConfig::default(),
        mcp: McpConfig {
            enabled: true,
            call_timeout_ms: 1_000,
            discovery_timeout_ms: 1_000,
            allowed_tools: vec![
                "test.echo".into(),
                "test.get_value".into(),
                "test.set_value".into(),
            ],
            ..McpConfig::default()
        },
    };
    let providers = Arc::new(ProviderSet::with_all(
        Arc::new(FakeVad),
        Arc::new(FakeAsr),
        Arc::new(ScriptedLlm),
        Arc::new(FakeTts),
    ));
    let app = router_with_providers(config, providers);
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{address}"), task)
}

#[tokio::test]
async fn reference_client_preserves_state_across_two_mcp_text_turns_and_receives_final_tts() {
    let (base, task) = start().await;
    let mut client = voice_reference_client::ReferenceClient::connect(
        voice_reference_client::McpSessionOptions {
            ota_url: format!("{base}/voice/ota/"),
            device_id: "phase6-reference".into(),
            client_id: "phase6-reference".into(),
            initial_value: 10,
            turn_timeout: Duration::from_secs(10),
        },
    )
    .await
    .unwrap();
    client.run_text_turn("Đặt giá trị thành 50").await.unwrap();
    client
        .run_text_turn("Giá trị hiện tại là bao nhiêu?")
        .await
        .unwrap();
    let report = client.finish();
    assert_eq!(report.initial_value, 10);
    assert_eq!(report.final_value, 50);
    assert_eq!(
        report.discovered_tools,
        vec!["test.echo", "test.get_value", "test.set_value"]
    );
    assert_eq!(report.received_calls.len(), 2);
    assert_eq!(report.received_calls[0].name, "test.set_value");
    assert_eq!(
        report.received_calls[0].arguments,
        serde_json::json!({"value":50})
    );
    assert!(!report.tool_results[0].is_error);
    assert_eq!(report.received_calls[1].name, "test.get_value");
    assert_eq!(report.tool_results[1].content, r#"{"value":50}"#);
    assert_eq!(
        report.final_assistant_text.as_deref(),
        Some("Giá trị hiện tại là 50.")
    );
    assert!(report.tts_started && report.tts_finished);
    task.abort();
}
