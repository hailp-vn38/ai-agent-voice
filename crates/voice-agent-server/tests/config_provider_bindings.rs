use reqwest::{Client, StatusCode};
use serde_json::{Value, json};
use std::{
    fs,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use voice_agent_server::{
    app::{AppState, router_with_state},
    config::{AppConfig, DatabaseConfig, DatabaseDevicesConfig},
    database::{Database, DesiredProvider},
    lifecycle::AdmissionGate,
    providers::ProviderSet,
    services::provider_runtime::{
        PreparedRuntime, ProviderRuntimeManager, RuntimeError, RuntimeLimits, RuntimeMaterializer,
        RuntimeResource,
    },
    workers::ProviderRuntimeAdmission,
};
struct Resource(voice_agent_server::providers::RuntimeCatalog);
impl RuntimeResource for Resource {
    fn runtimes(&self) -> Option<voice_agent_server::providers::RuntimeCatalog> {
        Some(self.0.clone())
    }
    fn unload(&self) -> bool {
        true
    }
    fn capabilities(&self) -> Option<Value> {
        Some(json!({"voices":["fixture"]}))
    }
}
struct Builder(
    Arc<AtomicUsize>,
    std::time::Duration,
    bool,
    Arc<voice_agent_server::workers::WorkerSupervisor>,
);
impl RuntimeMaterializer for Builder {
    fn estimated_peak_bytes(&self, _: &DesiredProvider) -> Result<u64, RuntimeError> {
        Ok(1)
    }
    fn logical_capacity(&self, _: &DesiredProvider) -> Result<usize, RuntimeError> {
        Ok(1)
    }
    fn build(
        &self,
        snapshot: &DesiredProvider,
        _: Option<PreparedRuntime>,
        _: ProviderRuntimeAdmission,
    ) -> Result<Arc<dyn RuntimeResource>, RuntimeError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        std::thread::sleep(self.1);
        use voice_agent_server::{
            providers::{DiagnosticRuntimeKind, ResolvedAgentRuntimes, RuntimeCatalog},
            workers::{
                AsrWorkerRuntime, LlmRuntime, TtsWorkerRuntime, VadWorkerRuntime,
                WorkerRuntimeConfig,
            },
        };
        let worker = WorkerRuntimeConfig {
            max_workers: 1,
            voice_reserved_capacity: 1,
            command_capacity: 4,
            final_timeout: std::time::Duration::from_secs(1),
            cleanup_grace: std::time::Duration::from_secs(1),
        };
        let runtimes = ResolvedAgentRuntimes {
            vad: Arc::new(VadWorkerRuntime::new(Arc::new(FixtureVad), worker.clone())),
            asr: Arc::new(AsrWorkerRuntime::new(Arc::new(FixtureAsr), worker.clone())),
            llm: Arc::new(LlmRuntime::new(
                Arc::new(FixtureLlm(snapshot.key.clone(), self.2)),
                1,
                std::time::Duration::from_secs(1),
            )),
            tts: Arc::new(TtsWorkerRuntime::new(Arc::new(FixtureTts), worker)),
            speaker: None,
            vad_segmenter: Default::default(),
            vad_pre_roll_samples: 0,
        };
        if snapshot.kind == "vad" {
            self.3.observe_vad(Arc::clone(&runtimes.vad));
        }
        let kind = match snapshot.kind.as_str() {
            "vad" => DiagnosticRuntimeKind::Vad,
            "asr" => DiagnosticRuntimeKind::Asr,
            "llm" => DiagnosticRuntimeKind::Llm,
            "tts" => DiagnosticRuntimeKind::Tts,
            _ => return Err(RuntimeError::Configuration),
        };
        Ok(Arc::new(Resource(RuntimeCatalog::single_provider(
            kind,
            snapshot.key.clone(),
            &runtimes,
        ))))
    }
}
async fn server(
    managed: bool,
) -> (
    String,
    Database,
    Arc<AtomicUsize>,
    tokio::task::JoinHandle<()>,
) {
    let (base, db, builds, task, _) =
        server_configured(managed, std::time::Duration::ZERO, false, 5000).await;
    (base, db, builds, task)
}
async fn server_configured(
    managed: bool,
    load_delay: std::time::Duration,
    hang_llm: bool,
    timeout_ms: u64,
) -> (
    String,
    Database,
    Arc<AtomicUsize>,
    tokio::task::JoinHandle<()>,
    Option<Arc<ProviderRuntimeManager>>,
) {
    let path = std::env::temp_dir().join(format!("config-bindings-{}.toml", uuid::Uuid::new_v4()));
    fs::write(
        &path,
        r#"
[server]
bind="127.0.0.1:0"
public_ws_url="ws://127.0.0.1:0/voice/v1/"
[provider_defaults]
vad="vad"
asr="asr"
llm="primary"
tts="primary"
[api]
enabled=true
admin_token="admin-test-token"
[providers.vad.instances.vad]
adapter="silero_onnx"
[providers.asr.instances.asr]
adapter="gipformer_sherpa_offline"
[providers.llm.instances.primary]
adapter="openai"
base_url="https://example.test/v1"
model="fixture"
api_key="SECRET_SENTINEL_SHOULD_NOT_APPEAR"
[providers.tts.instances.primary]
adapter="zerotts_onnx"
voice="maichi"
"#,
    )
    .unwrap();
    let mut config = AppConfig::parse_and_resolve(&path).unwrap();
    config.api.provider_tests.timeout_ms = timeout_ms;
    config.api.provider_tests.max_concurrency = 1;
    fs::remove_file(path).unwrap();
    let db = Database::connect(&DatabaseConfig {
        url: "sqlite::memory:".into(),
        max_connections: 1,
        ..Default::default()
    })
    .await
    .unwrap();
    sqlx::raw_sql(r#"
INSERT INTO providers (key,name,type,adapter,config_json,created_at,updated_at) VALUES ('primary','primary','llm','openai','{"base_url":"https://example.test/v1","model":"fixture"}',1,1),('db-tts','Z TTS','tts','openai','{"base_url":"https://example.test/v1","model":"tts"}',1,1);
INSERT INTO agent_templates (key,name,language,prompt,created_at,updated_at) VALUES ('template','Template','vi','hello',1,1);
INSERT INTO agents (key,name,created_at,updated_at) VALUES ('agent','Agent',1,1);
INSERT INTO devices (device_id,agent_id,created_at,updated_at) VALUES ('device',1,1,1);
INSERT INTO agent_template_assignments (agent_id,template_id,is_default,created_at) VALUES (1,1,1,1);
"#).execute(db.pool()).await.unwrap();
    let builds = Arc::new(AtomicUsize::new(0));
    let mut state = AppState::from_provider_set_with_database(
        config,
        Arc::new(ProviderSet::unavailable()),
        Some(db.clone()),
    );
    if managed {
        let manager = ProviderRuntimeManager::new(
            RuntimeLimits {
                max_parallel_loads: 1,
                max_pending_loads: 4,
                max_waiters: 16,
                max_resident_bytes: 40,
                max_resources: 8,
                max_version_entries: 16,
                admission_timeout_ms: 1000,
                failure_cooldown_ms: 100,
                idle_ttl_ms: 1000,
            },
            Arc::new(Builder(
                builds.clone(),
                load_delay,
                hang_llm,
                state.worker_supervisor.clone(),
            )),
            AdmissionGate::open(),
        )
        .unwrap();
        state = state.with_runtime_manager(manager);
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let manager = state.provider_runtime_manager.clone();
    let router = router_with_state(state);
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (
        format!("http://{address}/api/admin"),
        db,
        builds,
        task,
        manager,
    )
}
fn client() -> Client {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::AUTHORIZATION,
        "Bearer admin-test-token".parse().unwrap(),
    );
    Client::builder().default_headers(headers).build().unwrap()
}
async fn get(client: &Client, url: &str) -> Value {
    let response = client.get(url).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK, "{url}");
    let text = response.text().await.unwrap();
    assert!(!text.contains("SECRET_SENTINEL_SHOULD_NOT_APPEAR"));
    serde_json::from_str(&text).unwrap()
}
#[tokio::test]
async fn catalog_merges_sources_before_filtering_paging_and_never_loads_or_exposes_secrets() {
    let (base, _db, builds, task) = server(true).await;
    let c = client();
    let all = get(&c, &format!("{base}/providers?sort=name&page_size=4")).await;
    assert_eq!(all["total"], 7);
    assert_eq!(all["facets"]["vad"], 2);
    assert_eq!(all["total_pages"], 2);
    assert_eq!(all["facets"]["llm"], 2);
    assert_eq!(all["facets"]["tts"], 2);
    let second = get(
        &c,
        &format!("{base}/providers?sort=name&page_size=4&page=2"),
    )
    .await;
    let keys: std::collections::HashSet<_> = all["items"]
        .as_array()
        .unwrap()
        .iter()
        .chain(second["items"].as_array().unwrap())
        .map(|r| r["key"].as_str().unwrap())
        .collect();
    assert_eq!(keys.len(), 7);
    let filtered = get(
        &c,
        &format!("{base}/providers?type=llm&q=primary&enabled=true"),
    )
    .await;
    assert_eq!(filtered["total"], 2);
    assert_eq!(filtered["facets"]["tts"], 1);
    assert_eq!(
        get(&c, &format!("{base}/providers?enabled=false")).await["total"],
        0
    );
    let detail = get(&c, &format!("{base}/providers/config:llm:primary")).await;
    assert_eq!(detail["source"], "config");
    assert_eq!(detail["config_json"], "{}");
    assert_eq!(detail["runtime_status"], "not_loaded");
    assert_eq!(
        get(&c, &format!("{base}/providers/primary")).await["source"],
        "database"
    );
    assert_eq!(builds.load(Ordering::SeqCst), 0);
    for method in [reqwest::Method::PATCH, reqwest::Method::DELETE] {
        let r = c
            .request(method, format!("{base}/providers/config:llm:primary"))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::CONFLICT);
        assert!(r.text().await.unwrap().contains("config_provider_readonly"));
    }
    let before: (i64, i64, i64) = sqlx::query_as("SELECT (SELECT count(*) FROM providers), (SELECT revision FROM agent_templates WHERE key='template'), (SELECT count(*) FROM template_provider_bindings)").fetch_one(_db.pool()).await.unwrap();
    for payload in [
        json!({"name":"edited"}),
        json!({"config_json":{"model":"changed"}}),
        json!({"enabled":false}),
        json!({"api_key":"CANARY"}),
    ] {
        let response = c
            .patch(format!("{base}/providers/config:llm:primary"))
            .json(&payload)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = response.text().await.unwrap();
        assert!(body.contains("config_provider_readonly"));
        assert!(!body.contains("CANARY"));
    }
    let after: (i64, i64, i64) = sqlx::query_as("SELECT (SELECT count(*) FROM providers), (SELECT revision FROM agent_templates WHERE key='template'), (SELECT count(*) FROM template_provider_bindings)").fetch_one(_db.pool()).await.unwrap();
    assert_eq!(before, after);
    let r = c
        .post(format!("{base}/providers/config:tts:primary/test/llm"))
        .json(&json!({"input":"hello"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    assert!(r.text().await.unwrap().contains("provider_type_mismatch"));
    let r = c
        .get(format!("{base}/providers/config:llm:primary/capabilities"))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::CONFLICT);
    let r = c
        .post(format!("{base}/providers/config:llm:primary/prepare"))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(builds.load(Ordering::SeqCst), 1);
    assert_eq!(
        get(&c, &format!("{base}/providers/config:llm:primary")).await["runtime_status"],
        "loaded"
    );
    task.abort();
}
#[tokio::test]
async fn bindings_round_trip_replace_sources_preserve_revision_audit_and_admission() {
    let (base, db, _builds, task) = server(true).await;
    let c = client();
    for (revision, key) in [
        (1, "primary"),
        (2, "config:llm:primary"),
        (3, "primary"),
        (4, "config:llm:primary"),
    ] {
        let r = c
            .put(format!("{base}/templates/template/providers/llm"))
            .header("If-Match", format!("\"{revision}\""))
            .json(&json!({"provider_key":key}))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK, "{}", r.text().await.unwrap());
        let relationship = get(&c, &format!("{base}/templates/template/providers")).await;
        assert_eq!(relationship["revision"], revision + 1);
        assert_eq!(relationship["bindings"]["llm"]["provider_key"], key);
        let row:(String,Option<i64>,Option<String>)=sqlx::query_as("SELECT provider_source,provider_id,config_provider_key FROM template_provider_bindings").fetch_one(db.pool()).await.unwrap();
        if key.starts_with("config:") {
            assert_eq!(row, ("config".into(), None, Some("primary".into())));
        } else {
            assert_eq!(row, ("database".into(), Some(1), None));
        }
    }
    let usage = get(
        &c,
        &format!("{base}/providers/config:llm:primary/templates"),
    )
    .await;
    assert_eq!(usage["total"], 1);
    for key in [
        "config:tts:primary",
        "config:llm:missing",
        "config:llm:primary:extra",
    ] {
        let r = c
            .put(format!("{base}/templates/template/providers/llm"))
            .header("If-Match", "\"5\"")
            .json(&json!({"provider_key":key}))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    }
    let stale = c
        .put(format!("{base}/templates/template/providers/llm"))
        .header("If-Match", "\"1\"")
        .json(&json!({"provider_key":"primary"}))
        .send()
        .await
        .unwrap();
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    assert_eq!(
        get(&c, &format!("{base}/templates/template/providers")).await["revision"],
        5
    );
    let graph = db
        .admit_device("device", &DatabaseDevicesConfig::default())
        .await
        .unwrap();
    assert_eq!(
        graph.assignments[0].bindings[0].provider_key,
        "config:llm:primary"
    );
    assert!(graph.assignments[0].bindings[0].snapshot.is_none());
    sqlx::query("UPDATE template_provider_bindings SET config_provider_key='removed'")
        .execute(db.pool())
        .await
        .unwrap();
    let missing = get(&c, &format!("{base}/templates/template/providers")).await;
    assert_eq!(
        missing["bindings"]["llm"]["provider_key"],
        "config:llm:removed"
    );
    assert_eq!(missing["bindings"]["llm"]["enabled"], false);
    let r = c
        .delete(format!("{base}/templates/template/providers/llm"))
        .header("If-Match", "\"5\"")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(
        get(&c, &format!("{base}/templates/template/providers")).await["revision"],
        6
    );
    task.abort();
}
#[tokio::test]
async fn legacy_rejects_config_binding_before_mutating_template() {
    let (base, _db, _builds, task) = server(false).await;
    let c = client();
    let r = c
        .put(format!("{base}/templates/template/providers/llm"))
        .header("If-Match", "\"1\"")
        .json(&json!({"provider_key":"config:llm:primary"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::CONFLICT);
    assert!(
        r.text()
            .await
            .unwrap()
            .contains("config_provider_requires_managed_runtime")
    );
    assert_eq!(
        get(&c, &format!("{base}/templates/template/providers")).await["revision"],
        1
    );
    task.abort();
}

#[tokio::test]
async fn websocket_admits_mixed_sources_and_refuses_a_removed_explicit_config_binding() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::{
        connect_async,
        tungstenite::{Error, Message, client::IntoClientRequest},
    };
    let (base, db, _builds, task) = server(true).await;
    sqlx::raw_sql(r#"
INSERT INTO providers(key,name,type,adapter,config_json,created_at,updated_at) VALUES ('vad','VAD','vad','silero_onnx','{}',1,1),('asr','ASR','asr','gipformer_sherpa_offline','{"decoding_method":"greedy_search","max_active_paths":4}',1,1),('tts','TTS','tts','zerotts_onnx','{"voice":"maichi"}',1,1);
INSERT INTO template_provider_bindings(template_id,provider_type,provider_id,created_at,updated_at) SELECT 1,type,id,1,1 FROM providers WHERE key IN ('vad','asr','tts');
INSERT INTO template_provider_bindings(template_id,provider_type,provider_source,config_provider_key,created_at,updated_at) VALUES(1,'llm','config','primary',1,1);
"#).execute(db.pool()).await.unwrap();
    let request = || {
        let mut request = format!(
            "{}/voice/v1/",
            base.trim_end_matches("/api/admin")
                .replace("http://", "ws://")
        )
        .into_client_request()
        .unwrap();
        request
            .headers_mut()
            .insert("Protocol-Version", "1".parse().unwrap());
        request
            .headers_mut()
            .insert("Device-Id", "device".parse().unwrap());
        request
            .headers_mut()
            .insert("Client-Id", "config-bindings-test".parse().unwrap());
        request
    };
    let (mut socket, response) = connect_async(request()).await.unwrap();
    assert_eq!(response.status().as_u16(), 101);
    // Mutation affects a new admission, while the admitted session keeps its frozen profile.
    sqlx::query("UPDATE template_provider_bindings SET config_provider_key='removed' WHERE provider_source='config'").execute(db.pool()).await.unwrap();
    socket
        .send(Message::Text(r#"{"type":"hello"}"#.into()))
        .await
        .unwrap();
    let hello = tokio::time::timeout(std::time::Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(matches!(hello, Message::Text(_)));
    match connect_async(request()).await.unwrap_err() {
        Error::Http(r) => assert_eq!(r.status().as_u16(), 503),
        other => panic!("{other:?}"),
    }
    socket.close(None).await.unwrap();
    task.abort();
}

struct FixtureVad;
impl voice_agent_server::providers::VadProvider for FixtureVad {
    fn open(
        &self,
    ) -> Result<
        Box<dyn voice_agent_server::providers::VadSession>,
        voice_agent_server::providers::VadError,
    > {
        Ok(Box::new(FixtureVad))
    }
    fn adapter(&self) -> &'static str {
        "fixture"
    }
}
impl voice_agent_server::providers::VadSession for FixtureVad {
    fn push(
        &mut self,
        input: voice_agent_server::providers::VadInput,
    ) -> Result<
        voice_agent_server::providers::VadProbability,
        voice_agent_server::providers::VadError,
    > {
        Ok(voice_agent_server::providers::VadProbability {
            start_sample: input.start_sample,
            end_sample: input.start_sample + 512,
            probability: 0.25,
        })
    }
    fn reset(&mut self) -> Result<(), voice_agent_server::providers::VadError> {
        Ok(())
    }
}
struct FixtureAsr;
impl voice_agent_server::providers::AsrProvider for FixtureAsr {
    fn open(
        &self,
    ) -> Result<
        Box<dyn voice_agent_server::providers::AsrSession>,
        voice_agent_server::providers::AsrError,
    > {
        Ok(Box::new(FixtureAsr))
    }
}
impl voice_agent_server::providers::AsrSession for FixtureAsr {
    fn push_pcm(
        &mut self,
        _: &voice_agent_server::audio::PcmF32Mono,
    ) -> Result<Vec<voice_agent_server::providers::AsrEvent>, voice_agent_server::providers::AsrError>
    {
        Ok(Vec::new())
    }
    fn finish(
        &mut self,
    ) -> Result<voice_agent_server::providers::AsrResult, voice_agent_server::providers::AsrError>
    {
        Ok(voice_agent_server::providers::AsrResult::new("recognized"))
    }
    fn cancel(&mut self) {}
}

struct FixtureLlm(String, bool);
#[async_trait::async_trait]
impl voice_agent_server::providers::LlmProvider for FixtureLlm {
    async fn stream(
        &self,
        request: voice_agent_server::providers::llm::LlmRequest,
    ) -> Result<
        voice_agent_server::providers::llm::LlmEventStream,
        voice_agent_server::providers::LlmError,
    > {
        use voice_agent_server::providers::llm::LlmEvent;
        if self.1 {
            return Ok(Box::pin(futures_util::stream::pending()));
        }
        Ok(Box::pin(futures_util::stream::iter([
            Ok(LlmEvent::TextDelta(self.complete(&request)?)),
            Ok(LlmEvent::Finished),
        ])))
    }

    fn adapter(&self) -> &'static str {
        "fixture"
    }
    fn complete(
        &self,
        _: &voice_agent_server::providers::llm::LlmRequest,
    ) -> Result<String, voice_agent_server::providers::LlmError> {
        Ok(format!("answer:{}", self.0))
    }
}
struct FixtureTts;
impl voice_agent_server::providers::TtsProvider for FixtureTts {
    fn adapter(&self) -> &'static str {
        "fixture"
    }
    fn validate_diagnostic(
        &self,
        _: &voice_agent_server::providers::TtsDiagnosticRequest,
    ) -> Result<(), voice_agent_server::providers::TtsError> {
        Ok(())
    }
    fn synthesize_diagnostic(
        &self,
        _: &voice_agent_server::providers::TtsDiagnosticRequest,
        _: &std::sync::atomic::AtomicBool,
        on_pcm: &mut dyn FnMut(
            voice_agent_server::audio::PcmF32Mono,
        ) -> Result<(), voice_agent_server::providers::TtsError>,
    ) -> Result<(), voice_agent_server::providers::TtsError> {
        on_pcm(voice_agent_server::audio::PcmF32Mono::new(
            vec![0.0; 512],
            16_000,
        ))
    }
}

#[tokio::test]
async fn config_diagnostics_acquire_unbound_cold_deployment_and_reuse_prepare_without_db_collision()
{
    let (base, _db, builds, task) = server(true).await;
    let c = client();
    for key in ["primary", "config:llm:primary", "config:llm:primary"] {
        let response = c
            .post(format!("{base}/providers/{key}/test/llm"))
            .json(&json!({"input":"hello"}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let result: Value = response.json().await.unwrap();
        assert_eq!(result["provider_key"], key);
        assert_eq!(result["runtime"]["runtime_matches_desired"], true);
        assert_eq!(result["runtime"]["requires_restart"], false);
        assert_eq!(
            result["runtime"]["tested_provider_id"].is_null(),
            key.starts_with("config:")
        );
        assert!(!result.to_string().contains("SECRET_SENTINEL"));
        if key == "primary" {
            let cold = c
                .get(format!("{base}/providers/config:llm:primary/capabilities"))
                .send()
                .await
                .unwrap();
            assert_eq!(cold.status(), StatusCode::CONFLICT);
            assert!(
                cold.text()
                    .await
                    .unwrap()
                    .contains("provider_runtime_not_loaded")
            );
            assert_eq!(builds.load(Ordering::SeqCst), 1);
        }
    }
    let missing = c
        .post(format!("{base}/providers/config:llm:missing/test/llm"))
        .json(&json!({"input":"hello"}))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    assert!(missing.text().await.unwrap().contains("provider_not_found"));
    assert_eq!(builds.load(Ordering::SeqCst), 2);
    let response = c
        .post(format!("{base}/providers/config:llm:primary/prepare"))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(builds.load(Ordering::SeqCst), 2);
    let capabilities = get(
        &c,
        &format!("{base}/providers/config:llm:primary/capabilities"),
    )
    .await;
    assert_eq!(capabilities["source"], "config");
    assert_eq!(capabilities["capabilities"]["voices"][0], "fixture");
    assert_eq!(builds.load(Ordering::SeqCst), 2);
    let response = c
        .post(format!("{base}/providers/config:tts:primary/test/tts"))
        .json(&json!({"text":"hello", "language":"vi"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "audio/wav");
    assert_eq!(response.headers()["x-provider-key"], "config:tts:primary");
    assert!(!response.headers().contains_key("x-provider-tested-id"));
    assert!(response.bytes().await.unwrap().starts_with(b"RIFF"));
    task.abort();
}

#[tokio::test]
async fn legacy_config_diagnostic_requires_manager_and_never_uses_matching_db_key() {
    let (base, _db, builds, task) = server(false).await;
    let response = client()
        .post(format!("{base}/providers/config:llm:primary/test/llm"))
        .json(&json!({"input":"hello"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert!(
        response
            .text()
            .await
            .unwrap()
            .contains("config_provider_requires_managed_runtime")
    );
    assert_eq!(builds.load(Ordering::SeqCst), 0);
    task.abort();
}

#[tokio::test]
async fn config_vad_and_asr_keep_validation_and_public_provenance() {
    let (base, _db, builds, task) = server(true).await;
    let c = client();
    let result = c
        .post(format!("{base}/providers/config:vad:vad/test/vad"))
        .send()
        .await
        .unwrap();
    assert_eq!(result.status(), StatusCode::OK);
    let result: Value = result.json().await.unwrap();
    assert_eq!(result["provider_key"], "config:vad:vad");
    assert_eq!(result["source"], "config");
    assert_eq!(result["result"]["probability"], 0.25);
    let bad = c
        .post(format!("{base}/providers/config:vad:vad/test/vad"))
        .body("x")
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
    let mut wav = std::io::Cursor::new(Vec::new());
    let mut writer = hound::WavWriter::new(
        &mut wav,
        hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )
    .unwrap();
    for _ in 0..1600 {
        writer.write_sample(0i16).unwrap();
    }
    writer.finalize().unwrap();
    let result = c
        .post(format!("{base}/providers/config:asr:asr/test/asr"))
        .header("content-type", "audio/wav")
        .body(wav.into_inner())
        .send()
        .await
        .unwrap();
    assert_eq!(result.status(), StatusCode::OK);
    let result: Value = result.json().await.unwrap();
    assert_eq!(result["provider_key"], "config:asr:asr");
    assert_eq!(result["result"]["text"], "recognized");
    assert_eq!(result["runtime"]["tested_provider_id"], Value::Null);
    assert_eq!(builds.load(Ordering::SeqCst), 2);
    task.abort();
}

#[tokio::test]
async fn config_cold_acquisition_is_bounded_and_busy_without_releasing_the_native_attempt() {
    let (base, _db, builds, task, manager) =
        server_configured(true, std::time::Duration::from_millis(300), false, 100).await;
    let manager = manager.unwrap();
    let c = client();
    let url = format!("{base}/providers/config:llm:primary/test/llm");
    let pending = tokio::spawn({
        let c = c.clone();
        let url = url.clone();
        async move {
            c.post(url)
                .json(&json!({"input":"hello"}))
                .send()
                .await
                .unwrap()
        }
    });
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while builds.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    let busy = c
        .post(&url)
        .json(&json!({"input":"hello"}))
        .send()
        .await
        .unwrap();
    assert_eq!(busy.status(), StatusCode::TOO_MANY_REQUESTS);
    let timed_out = pending.await.unwrap();
    assert_eq!(timed_out.status(), StatusCode::GATEWAY_TIMEOUT);
    assert!(manager.accounting().reserved_bytes > 0);
    assert_eq!(builds.load(Ordering::SeqCst), 1);
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while manager.accounting().reserved_build_bytes > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let result = c
        .post(&url)
        .json(&json!({"input":"hello"}))
        .send()
        .await
        .unwrap();
    assert_eq!(result.status(), StatusCode::OK);
    assert_eq!(builds.load(Ordering::SeqCst), 1);
    task.abort();
}

#[tokio::test]
async fn config_operation_timeout_quarantines_deployment_lease_without_terminal_ack() {
    let (base, _db, builds, task, manager) =
        server_configured(true, std::time::Duration::ZERO, true, 100).await;
    let manager = manager.unwrap();
    let c = client();
    let url = format!("{base}/providers/config:llm:primary/test/llm");
    let response = c
        .post(&url)
        .json(&json!({"input":"hello"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
    assert_eq!(manager.accounting().active_leases, 1);
    let busy = c
        .post(&url)
        .json(&json!({"input":"hello"}))
        .send()
        .await
        .unwrap();
    assert_eq!(busy.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(manager.accounting().active_leases, 1);
    assert_eq!(builds.load(Ordering::SeqCst), 1);
    task.abort();
}
