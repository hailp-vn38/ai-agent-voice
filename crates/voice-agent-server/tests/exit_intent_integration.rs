mod support;

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use futures_util::{SinkExt, StreamExt, stream};
use tokio::{net::TcpListener, task::JoinHandle, time::timeout};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};
use url::Url;
use voice_agent_server::{
    audio::PcmF32Mono,
    config::{
        AppConfig, AudioConfig, AuthConfig, BargeInConfig, DeploymentConfig, LimitsConfig,
        LlmConfig, McpConfig, ProvidersConfig, RuntimeConfig, ServerConfig, SpeechOutputConfig,
        TtsConfig, WebsocketConfig, WorkersConfig,
    },
    providers::{
        AsrError, AsrEvent, AsrProvider, AsrResult, AsrSession, LlmError, LlmEvent, LlmProvider,
        ProviderSet, TtsError, TtsProvider, VadError, VadInput, VadProbability, VadProvider,
        VadSession,
        llm::{LlmEventStream, LlmRequest, ToolCall},
    },
    tools::builtin::EXIT_TOOL_NAME,
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
struct ExitLlm(Arc<AtomicBool>);
#[async_trait::async_trait]
impl LlmProvider for ExitLlm {
    fn adapter(&self) -> &'static str {
        "exit-scripted"
    }
    async fn stream(&self, request: LlmRequest) -> Result<LlmEventStream, LlmError> {
        self.0.store(
            request.tools.iter().any(|tool| tool.name == EXIT_TOOL_NAME),
            Ordering::SeqCst,
        );
        Ok(Box::pin(stream::iter(vec![
            Ok(LlmEvent::ToolCall(ToolCall {
                id: "exit-1".into(),
                name: EXIT_TOOL_NAME.into(),
                arguments: serde_json::json!({"say_goodbye":"Tạm biệt, hẹn gặp lại!"}),
            })),
            Ok(LlmEvent::Finished),
        ])))
    }
}
struct FakeTts;
impl TtsProvider for FakeTts {
    fn adapter(&self) -> &'static str {
        "fake"
    }
    fn synthesize(&self, _: &str) -> Result<PcmF32Mono, TtsError> {
        Ok(PcmF32Mono::new(vec![0.1; 96_000], 48_000))
    }
}

#[tokio::test]
async fn aborting_the_goodbye_cancels_the_pending_normal_close() {
    let tool_was_advertised = Arc::new(AtomicBool::new(false));
    let (base, task) = start(tool_was_advertised).await;
    let ota: serde_json::Value = reqwest::Client::new()
        .post(format!("{base}/voice/ota/"))
        .header("Device-Id", "exit-abort")
        .header("Client-Id", "exit-abort")
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let mut request = ota["websocket"]["url"]
        .as_str()
        .unwrap()
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("Protocol-Version", "1".parse().unwrap());
    request
        .headers_mut()
        .insert("Device-Id", "exit-abort".parse().unwrap());
    request
        .headers_mut()
        .insert("Client-Id", "exit-abort".parse().unwrap());
    let (mut socket, _) = connect_async(request).await.unwrap();
    socket
        .send(Message::Text(
            serde_json::json!({"type":"hello","version":1,"transport":"websocket","features":{"mcp":false},"audio_params":{"format":"opus","sample_rate":16000,"channels":1,"frame_duration":60}})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    let hello = timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let Message::Text(hello) = hello else {
        panic!("expected server hello")
    };
    let session_id = serde_json::from_str::<serde_json::Value>(&hello).unwrap()["session_id"]
        .as_str()
        .unwrap()
        .to_owned();
    socket
        .send(Message::Text(
            serde_json::json!({"type":"listen","session_id":session_id,"state":"start","mode":"manual"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    socket
        .send(Message::Text(
            serde_json::json!({"type":"listen","session_id":session_id,"state":"detect","text":"Tạm biệt"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    loop {
        let frame = timeout(Duration::from_secs(2), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if matches!(frame, Message::Text(ref text) if serde_json::from_str::<serde_json::Value>(text).ok().as_ref().is_some_and(|value| value["type"] == "tts" && value["state"] == "start"))
        {
            break;
        }
    }
    socket
        .send(Message::Text(
            serde_json::json!({"type":"abort","session_id":session_id})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    let mut saw_abort_stop = false;
    let mut saw_normal_close = false;
    while let Ok(Some(Ok(frame))) = timeout(Duration::from_millis(500), socket.next()).await {
        saw_abort_stop |= matches!(frame, Message::Text(ref text) if serde_json::from_str::<serde_json::Value>(text).ok().as_ref().is_some_and(|value| value["type"] == "tts" && value["state"] == "stop"));
        saw_normal_close |=
            matches!(frame, Message::Close(Some(ref close)) if u16::from(close.code) == 1000);
    }
    assert!(saw_abort_stop, "abort must stop the goodbye playback");
    assert!(!saw_normal_close, "aborted goodbye must not close normally");
    task.abort();
}

async fn start(tool_was_advertised: Arc<AtomicBool>) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let config = AppConfig {
        speaker_recognition: Default::default(),
        server: ServerConfig {
            bind: address,
            public_ws_url: Url::parse(&format!("ws://{address}/voice/v1/")).unwrap(),
            hello_timeout_ms: 500,
        },
        auth: AuthConfig::default(),
        audio: AudioConfig::default(),
        websocket: WebsocketConfig::default(),
        limits: LimitsConfig::default(),
        provider_defaults: voice_agent_server::config::ProviderDefaultsConfig {
            vad: "test".into(),
            asr: "test".into(),
            llm: "test".into(),
            tts: "test".into(),
            vision: None,
        },
        providers: ProvidersConfig::default(),
        workers: WorkersConfig::default(),
        deployment: DeploymentConfig::default(),
        runtime: RuntimeConfig::default(),
        provider_runtime: None,
        llm: LlmConfig::default(),
        tts: TtsConfig::default(),
        speech_output: SpeechOutputConfig::default(),
        barge_in: BargeInConfig::default(),
        mcp: McpConfig {
            enabled: false,
            ..McpConfig::default()
        },
        vision: voice_agent_server::config::VisionConfig::default(),
        database: voice_agent_server::config::DatabaseConfig::default(),
        api: voice_agent_server::config::AdminApiConfig::default(),
        shutdown: voice_agent_server::config::ShutdownConfig::default(),
        agent: None,
        effective_agent: voice_agent_server::config::EffectiveAgentConfig::default(),
    };
    let providers = Arc::new(ProviderSet::with_all(
        Arc::new(FakeVad),
        Arc::new(FakeAsr),
        Arc::new(ExitLlm(tool_was_advertised)),
        Arc::new(FakeTts),
    ));
    let app = support::router(config, providers).await;
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{address}"), task)
}

#[tokio::test]
async fn exit_builtin_without_mcp_sends_final_audio_then_stop_then_normal_close() {
    let tool_was_advertised = Arc::new(AtomicBool::new(false));
    let (base, task) = start(Arc::clone(&tool_was_advertised)).await;
    let ota: serde_json::Value = reqwest::Client::new()
        .post(format!("{base}/voice/ota/"))
        .header("Device-Id", "exit-gate")
        .header("Client-Id", "exit-gate")
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let mut request = ota["websocket"]["url"]
        .as_str()
        .unwrap()
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("Protocol-Version", "1".parse().unwrap());
    request
        .headers_mut()
        .insert("Device-Id", "exit-gate".parse().unwrap());
    request
        .headers_mut()
        .insert("Client-Id", "exit-gate".parse().unwrap());
    let (mut socket, _) = connect_async(request).await.unwrap();
    socket.send(Message::Text(serde_json::json!({"type":"hello","version":1,"transport":"websocket","features":{"mcp":false},"audio_params":{"format":"opus","sample_rate":16000,"channels":1,"frame_duration":60}}).to_string().into())).await.unwrap();
    let hello = timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let Message::Text(hello) = hello else {
        panic!("expected server hello")
    };
    let session_id = serde_json::from_str::<serde_json::Value>(&hello).unwrap()["session_id"]
        .as_str()
        .unwrap()
        .to_owned();
    socket.send(Message::Text(serde_json::json!({"type":"listen","session_id":session_id,"state":"start","mode":"manual"}).to_string().into())).await.unwrap();
    socket.send(Message::Text(serde_json::json!({"type":"listen","session_id":session_id,"state":"detect","text":"Tạm biệt"}).to_string().into())).await.unwrap();

    let mut frames = Vec::new();
    loop {
        let frame = timeout(Duration::from_secs(5), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let is_close = matches!(frame, Message::Close(_));
        frames.push(frame);
        if is_close {
            break;
        }
    }
    assert!(
        tool_was_advertised.load(Ordering::SeqCst),
        "builtin tool must be offered even with MCP disabled"
    );
    let last_audio = frames
        .iter()
        .rposition(|frame| matches!(frame, Message::Binary(_)))
        .expect("goodbye audio");
    let stop = frames.iter().position(|frame| matches!(frame, Message::Text(text) if serde_json::from_str::<serde_json::Value>(text).ok().as_ref().is_some_and(|value| value["type"] == "tts" && value["state"] == "stop"))).expect("tts stop");
    let close = frames
        .iter()
        .position(
            |frame| matches!(frame, Message::Close(Some(close)) if u16::from(close.code) == 1000),
        )
        .expect("normal close");
    assert!(
        last_audio < stop && stop < close,
        "final audio, tts:stop, and Close(1000) must stay ordered"
    );
    task.abort();
}
