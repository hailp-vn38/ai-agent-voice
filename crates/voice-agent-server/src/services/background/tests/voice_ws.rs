use super::*;
use crate::{
    app::{AppState, router_with_state},
    providers::{
        LlmError, LlmEvent, LlmProvider, ProviderSet,
        llm::{ChatMessage, LlmEventStream, LlmRequest, ToolCall},
    },
    tools::builtin::CREATE_JOB_TOOL_NAME,
};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};

struct VoiceModel {
    database: Arc<Database>,
    receipt: tokio::sync::Mutex<Option<Value>>,
    received: tokio::sync::Notify,
}
struct Capture;
impl crate::providers::AsrProvider for Capture {
    fn open(&self) -> Result<Box<dyn crate::providers::AsrSession>, crate::providers::AsrError> {
        Ok(Box::new(Capture))
    }
}
impl crate::providers::AsrSession for Capture {
    fn push_pcm(
        &mut self,
        _: &crate::audio::PcmF32Mono,
    ) -> Result<Vec<crate::providers::AsrEvent>, crate::providers::AsrError> {
        Ok(Vec::new())
    }
    fn finish(&mut self) -> Result<crate::providers::AsrResult, crate::providers::AsrError> {
        Ok(crate::providers::AsrResult::new(""))
    }
    fn cancel(&mut self) {}
    fn reset(&mut self) -> Result<(), crate::providers::AsrError> {
        Ok(())
    }
}
#[async_trait::async_trait]
impl LlmProvider for VoiceModel {
    fn adapter(&self) -> &'static str {
        "p3-voice-fixture"
    }
    fn supports_tools(&self) -> bool {
        true
    }
    async fn stream(&self, request: LlmRequest) -> Result<LlmEventStream, LlmError> {
        assert!(
            request
                .tools
                .iter()
                .any(|tool| tool.name == CREATE_JOB_TOOL_NAME)
        );
        if let Some(content) = request.messages.iter().find_map(|message| match message {
            ChatMessage::ToolResult {
                tool_call_id,
                content,
            } if tool_call_id == "job-call" => Some(content),
            _ => None,
        }) {
            let receipt: Value = serde_json::from_str(content).unwrap();
            assert_eq!(receipt["created"], true);
            let committed = self
                .database
                .job_view(receipt["job_id"].as_str().unwrap())
                .await
                .unwrap();
            assert!(
                committed.requester_scope.starts_with("voice:"),
                "LLM continuation must receive a committed Voice receipt"
            );
            *self.receipt.lock().await = Some(receipt);
            self.received.notify_one();
            Ok(Box::pin(futures_util::stream::iter([
                Ok(LlmEvent::TextDelta("Job queued".into())),
                Ok(LlmEvent::Finished),
            ])))
        } else {
            Ok(Box::pin(futures_util::stream::iter([
                Ok(LlmEvent::ToolCall(ToolCall {
                    id: "job-call".into(),
                    name: CREATE_JOB_TOOL_NAME.into(),
                    arguments: json!({"goal":"Voice WS source tracer"}),
                })),
                Ok(LlmEvent::Finished),
            ])))
        }
    }
}

#[tokio::test]
async fn admitted_physical_ws_without_mcp_creates_durable_job_and_disconnect_does_not_replay() {
    let config = Arc::new(config());
    let db = Arc::new(Database::connect(&config.database).await.unwrap());
    voice(&db).await;
    let runtime = BackgroundRuntime::execution_harness(
        db.clone(),
        config.clone(),
        Arc::new(super::delegation::SessionModels),
    )
    .await
    .unwrap();
    enable(&runtime.engine).await;
    let model = Arc::new(VoiceModel {
        database: db.clone(),
        receipt: Default::default(),
        received: Default::default(),
    });
    let providers = Arc::new(ProviderSet::with_all(
        Arc::new(crate::providers::vad::UnavailableVad),
        Arc::new(Capture),
        model.clone(),
        Arc::new(crate::providers::tts::UnavailableTts),
    ));
    let mut state = AppState::from_provider_set_with_database(
        (*config).clone(),
        providers,
        Some((*db).clone()),
    );
    state.background = Some(runtime.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, router_with_state(state))
            .await
            .unwrap()
    });
    let mut request = format!("ws://{address}/voice/v1/")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("Device-Id", "voice-device".parse().unwrap());
    request
        .headers_mut()
        .insert("Client-Id", "p3-source-client".parse().unwrap());
    request
        .headers_mut()
        .insert("Protocol-Version", "1".parse().unwrap());
    let (mut socket, _) = connect_async(request.clone()).await.unwrap();
    let hello = json!({"type":"hello","features":{"mcp":false}}).to_string();
    socket
        .send(Message::Text(hello.clone().into()))
        .await
        .unwrap();
    let greeting = tokio::time::timeout(std::time::Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(matches!(greeting, Message::Text(_)));
    assert_eq!(runtime.engine.list(1, 30, None, None).await.unwrap().1, 0);
    socket
        .send(Message::Text(
            json!({"type":"listen","state":"start","mode":"manual"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    socket
        .send(Message::Text(
            json!({"type":"listen","state":"detect","text":"Please research in background"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(3), model.received.notified())
        .await
        .unwrap();
    let receipt = model.receipt.lock().await.clone().unwrap();
    let id = receipt["job_id"].as_str().unwrap();
    socket.close(None).await.unwrap();
    let job = wait_for(&runtime.engine, id, |job| job.status == "succeeded").await;
    assert!(job.requester_scope.starts_with("voice:"));
    let response = reqwest::Client::new()
        .get(format!("http://{address}/api/admin/jobs/{id}"))
        .bearer_auth("background-test-token")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.json::<Value>().await.unwrap()["status"],
        "succeeded"
    );
    let (mut reconnect, _) =
        tokio::time::timeout(std::time::Duration::from_secs(3), connect_async(request))
            .await
            .unwrap()
            .unwrap();
    reconnect.send(Message::Text(hello.into())).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), reconnect.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    reconnect.close(None).await.unwrap();
    assert_eq!(runtime.engine.list(1, 30, None, None).await.unwrap().1, 1);
    server.abort();
    runtime.shutdown().await;
}
