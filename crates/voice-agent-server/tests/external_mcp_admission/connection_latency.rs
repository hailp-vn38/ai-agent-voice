use super::*;
use futures_util::{SinkExt, StreamExt};
use voice_agent_server::{
    database::DesiredProvider,
    providers::{DiagnosticRuntimeKind, ResolvedAgentRuntimes, RuntimeCatalog},
    services::provider_runtime::{
        PreparedRuntime, RuntimeError, RuntimeMaterializer, RuntimeResource,
    },
    workers::ProviderRuntimeAdmission,
};

struct GatedMaterializer {
    runtimes: ResolvedAgentRuntimes,
    entered: tokio::sync::mpsc::UnboundedSender<String>,
    release: Mutex<std::sync::mpsc::Receiver<()>>,
    fail: bool,
}
struct Resource(RuntimeCatalog);
impl RuntimeResource for Resource {
    fn unload(&self) -> bool {
        true
    }
    fn runtimes(&self) -> Option<RuntimeCatalog> {
        Some(self.0.clone())
    }
}
impl RuntimeMaterializer for GatedMaterializer {
    fn estimated_peak_bytes(&self, _: &DesiredProvider) -> Result<u64, RuntimeError> {
        Ok(1)
    }
    fn logical_capacity(&self, _: &DesiredProvider) -> Result<usize, RuntimeError> {
        Ok(1)
    }
    fn build(
        &self,
        row: &DesiredProvider,
        _: Option<PreparedRuntime>,
        _: ProviderRuntimeAdmission,
    ) -> Result<Arc<dyn RuntimeResource>, RuntimeError> {
        let kind = match row.kind.as_str() {
            "asr" => {
                self.entered.send(row.key.clone()).unwrap();
                self.release.lock().unwrap().recv().unwrap();
                if self.fail {
                    return Err(RuntimeError::Unavailable);
                }
                DiagnosticRuntimeKind::Asr
            }
            "vad" => DiagnosticRuntimeKind::Vad,
            "llm" => DiagnosticRuntimeKind::Llm,
            "tts" => DiagnosticRuntimeKind::Tts,
            _ => unreachable!(),
        };
        Ok(Arc::new(Resource(RuntimeCatalog::single_provider(
            kind,
            row.key.clone(),
            &self.runtimes,
        ))))
    }
}

async fn seed_template(pool: &SqlitePool) {
    seed(pool).await;
    sqlx::raw_sql("INSERT INTO agent_templates (key,name,language,prompt,created_at,updated_at) VALUES ('primary','Primary','vi-VN','prompt',1,1); INSERT INTO agent_template_assignments (agent_id,template_id,is_default,created_at) VALUES (1,1,1,1);")
        .execute(pool).await.unwrap();
    for (kind, adapter, config) in [
        ("vad", "silero_onnx", "{}"),
        ("asr", "zipformer_sherpa", "{}"),
        (
            "llm",
            "openai",
            r#"{"base_url":"http://example.test/v1","model":"fixture"}"#,
        ),
        ("tts", "zerotts_onnx", r#"{"voice":"maichi"}"#),
    ] {
        let id: i64 = sqlx::query_scalar("INSERT INTO providers (key,name,type,adapter,config_json,created_at,updated_at) VALUES (?,?,?,?,?,1,1) RETURNING id")
            .bind(kind).bind(kind).bind(kind).bind(adapter).bind(config).fetch_one(pool).await.unwrap();
        sqlx::query("INSERT INTO template_provider_bindings (template_id,provider_type,provider_id,created_at,updated_at) VALUES (1,?,?,1,1)")
            .bind(kind).bind(id).execute(pool).await.unwrap();
    }
}

async fn gated_voice(
    fail: bool,
) -> (
    Voice,
    tokio::sync::mpsc::UnboundedReceiver<String>,
    std::sync::mpsc::Sender<()>,
) {
    let mut app_config = config(database_url());
    app_config.api.enabled = true;
    app_config.api.admin_token = "latency-admin-token".into();
    app_config.database.devices.enrollment.enabled = true;
    let fixture = AppState::from_provider_set(
        app_config.clone(),
        Arc::new(ProviderSet::with_all(
            Arc::new(SilentVad),
            Arc::new(FinalAsr),
            Arc::new(EchoLlm),
            Arc::new(ShortTts),
        )),
    );
    let runtimes = fixture
        .runtimes
        .resolve(&fixture.config.provider_defaults.effective_bindings())
        .unwrap();
    let (entered, entered_rx) = tokio::sync::mpsc::unbounded_channel();
    let (release, release_rx) = std::sync::mpsc::channel();
    let voice = start_with_materializer(
        app_config,
        ConstantSecrets(None),
        Some(Arc::new(GatedMaterializer {
            runtimes,
            entered,
            release: Mutex::new(release_rx),
            fail,
        })),
    )
    .await;
    seed_template(&voice.database().await).await;
    (voice, entered_rx, release)
}

fn request(voice: &Voice) -> tokio_tungstenite::tungstenite::handshake::client::Request {
    let mut request = format!("{}/voice/v1/", voice.base.replacen("http", "ws", 1))
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("Device-Id", "device".parse().unwrap());
    request
        .headers_mut()
        .insert("Client-Id", "latency-test".parse().unwrap());
    request
}

#[tokio::test]
async fn websocket_discovers_mcp_while_selected_runtime_is_still_cold() {
    let server = start_mcp(McpBehaviour::with_tools(vec![tool("Light")])).await;
    let (voice, mut entered, release) = gated_voice(false).await;
    bind_server(
        &voice.database().await,
        "external",
        &server.url(),
        "{}",
        ("none", None, None),
    )
    .await;
    let admission = tokio::spawn(connect_async(request(&voice)));
    timeout(Duration::from_secs(2), entered.recv())
        .await
        .unwrap()
        .unwrap();
    let discovery = timeout(Duration::from_millis(500), async {
        while !server
            .behaviour
            .lock()
            .unwrap()
            .sessions
            .lock()
            .unwrap()
            .iter()
            .any(|(method, _)| method == "tools/list")
        {
            tokio::task::yield_now().await;
        }
    })
    .await;
    release.send(()).unwrap();
    let (mut socket, _) = admission.await.unwrap().unwrap();
    socket
        .send(tokio_tungstenite::tungstenite::Message::Text(
            r#"{"type":"hello","version":1,"transport":"websocket"}"#.into(),
        ))
        .await
        .unwrap();
    let hello = timeout(Duration::from_secs(1), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let hello: serde_json::Value = serde_json::from_str(hello.to_text().unwrap()).unwrap();
    assert_eq!(hello["type"], "hello");
    let metrics = voice
        .state
        .provider_runtime_manager
        .as_ref()
        .unwrap()
        .metrics()
        .clone();
    timeout(Duration::from_secs(1), async {
        while metrics.snapshot()["duration_ms"]["phases"]["ws_connect"]["count"] != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    for phase in [
        "ws_routing",
        "ws_runtime",
        "ws_external_mcp",
        "ws_speaker",
        "ws_pre_upgrade",
        "ws_hello_wait",
        "ws_hello_init",
        "ws_hello_send",
    ] {
        assert_eq!(
            metrics.snapshot()["duration_ms"]["phases"][phase]["count"],
            1,
            "{phase}"
        );
    }
    voice.task.abort();
    server.task.abort();
    assert!(
        discovery.is_ok(),
        "External MCP must complete discovery before the cold runtime is released"
    );
}

#[tokio::test]
async fn a_failed_runtime_rejects_upgrade_without_waiting_for_optional_mcp() {
    let gate = Arc::new(tokio::sync::Notify::new());
    let server = start_mcp(McpBehaviour {
        initialize_gate: Some(gate),
        ..McpBehaviour::with_tools(vec![tool("Light")])
    })
    .await;
    let (voice, mut entered, release) = gated_voice(true).await;
    bind_server(
        &voice.database().await,
        "external",
        &server.url(),
        "{}",
        ("none", None, None),
    )
    .await;
    let admission = tokio::spawn(connect_async(request(&voice)));
    timeout(Duration::from_secs(2), entered.recv())
        .await
        .unwrap()
        .unwrap();
    release.send(()).unwrap();
    let error = timeout(Duration::from_millis(500), admission)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(
        matches!(error, tokio_tungstenite::tungstenite::Error::Http(response) if response.status() == WsStatusCode::SERVICE_UNAVAILABLE)
    );
    voice.task.abort();
    server.task.abort();
}

async fn seed_override(pool: &SqlitePool) {
    sqlx::raw_sql(r#"
INSERT INTO agent_templates (id,key,name,language,prompt,created_at,updated_at) VALUES (2,'override','Override','vi-VN','prompt',1,1);
INSERT INTO agent_template_assignments (agent_id,template_id,is_default,created_at) VALUES (1,2,0,1);
INSERT INTO providers (id,key,name,type,adapter,config_json,created_at,updated_at) VALUES (5,'override_asr','Override ASR','asr','zipformer_sherpa','{}',1,1);
INSERT INTO template_provider_bindings (template_id,provider_type,provider_id,created_at,updated_at)
SELECT 2,provider_type,CASE WHEN provider_type='asr' THEN 5 ELSE provider_id END,1,1 FROM template_provider_bindings WHERE template_id=1;
"#).execute(pool).await.unwrap();
}

async fn assert_override_warmed(
    voice: &Voice,
    entered: &mut tokio::sync::mpsc::UnboundedReceiver<String>,
    release: std::sync::mpsc::Sender<()>,
) {
    let built = timeout(Duration::from_secs(2), entered.recv())
        .await
        .unwrap()
        .unwrap();
    release.send(()).unwrap();
    assert_eq!(built, "override_asr");
    let manager = voice.state.provider_runtime_manager.as_ref().unwrap();
    timeout(Duration::from_secs(2), async {
        while manager.inspect(5, 1).desired_state
            != voice_agent_server::services::provider_runtime::RuntimeState::Ready
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        manager.inspect(2, 1).desired_state,
        voice_agent_server::services::provider_runtime::RuntimeState::Cold,
        "prewarm must not load the default ASR for an explicit override"
    );
    voice.task.abort();
}

#[tokio::test]
async fn creating_a_device_prewarms_its_override_before_its_first_websocket() {
    let (voice, mut entered, release) = gated_voice(false).await;
    seed_override(&voice.database().await).await;
    let response = reqwest::Client::new().post(format!("{}/api/admin/devices", voice.base)).bearer_auth("latency-admin-token")
        .json(&serde_json::json!({"device_id":"created", "agent_key":"agent", "template_key":"override"})).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_override_warmed(&voice, &mut entered, release).await;
}

#[tokio::test]
async fn updating_a_device_prewarms_its_new_override_without_a_websocket() {
    let (voice, mut entered, release) = gated_voice(false).await;
    seed_override(&voice.database().await).await;
    let response = reqwest::Client::new()
        .patch(format!("{}/api/admin/devices/device", voice.base))
        .bearer_auth("latency-admin-token")
        .header("If-Match", "\"1\"")
        .json(&serde_json::json!({"template_key":"override"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_override_warmed(&voice, &mut entered, release).await;
}

#[tokio::test]
async fn claiming_enrollment_prewarms_the_selected_override_before_reconnect() {
    let (voice, mut entered, release) = gated_voice(false).await;
    seed_override(&voice.database().await).await;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    voice
        .state
        .database
        .as_ref()
        .unwrap()
        .get_or_create_enrollment(
            voice_agent_server::database::EnrollmentRequest {
                device_id: "enrolled".into(),
                client_id: "latency-test".into(),
                metadata_json: "{}".into(),
                now,
                ttl_seconds: 600,
                candidates: vec![("123456".into(), uuid::Uuid::new_v4().to_string())],
            },
            100,
        )
        .await
        .unwrap();
    let response = reqwest::Client::new()
        .post(format!("{}/api/admin/device-enrollments/claim", voice.base))
        .bearer_auth("latency-admin-token")
        .json(&serde_json::json!({"code":"123456", "agent_key":"agent", "template_key":"override"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_override_warmed(&voice, &mut entered, release).await;
}
