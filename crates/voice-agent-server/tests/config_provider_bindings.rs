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
}
struct Builder(Arc<AtomicUsize>);
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
        use voice_agent_server::{
            providers::{
                DiagnosticRuntimeKind, ResolvedAgentRuntimes, RuntimeCatalog, llm::UnavailableLlm,
                tts::UnavailableTts,
            },
            workers::{
                AsrWorkerRuntime, LlmRuntime, TtsWorkerRuntime, VadWorkerRuntime,
                WorkerRuntimeConfig,
            },
        };
        let worker = WorkerRuntimeConfig {
            max_workers: 1,
            voice_reserved_capacity: 1,
            command_capacity: 1,
            final_timeout: std::time::Duration::from_secs(1),
            cleanup_grace: std::time::Duration::from_secs(1),
        };
        let runtimes = ResolvedAgentRuntimes {
            vad: Arc::new(VadWorkerRuntime::new(Arc::new(FixtureVad), worker.clone())),
            asr: Arc::new(AsrWorkerRuntime::new(Arc::new(FixtureAsr), worker.clone())),
            llm: Arc::new(LlmRuntime::new(
                Arc::new(UnavailableLlm),
                1,
                std::time::Duration::from_secs(1),
            )),
            tts: Arc::new(TtsWorkerRuntime::new(Arc::new(UnavailableTts), worker)),
            speaker: None,
            vad_segmenter: Default::default(),
            vad_pre_roll_samples: 0,
        };
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
[providers.llm.instances.primary]
adapter="openai"
base_url="https://example.test/v1"
model="fixture"
api_key="SECRET_SENTINEL_SHOULD_NOT_APPEAR"
[providers.tts.instances.primary]
adapter="vieneu_http"
base_url="https://example.test/v1"
voice="Mai Anh"
"#,
    )
    .unwrap();
    let config = AppConfig::parse_and_resolve(&path).unwrap();
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
            Arc::new(Builder(builds.clone())),
            AdmissionGate::open(),
        )
        .unwrap();
        state = state.with_runtime_manager(manager);
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = router_with_state(state);
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (format!("http://{address}/api/admin"), db, builds, task)
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
    let all = get(&c, &format!("{base}/providers?sort=name&page_size=3")).await;
    assert_eq!(all["total"], 5);
    assert_eq!(all["facets"]["vad"], 1);
    assert_eq!(all["total_pages"], 2);
    assert_eq!(all["facets"]["llm"], 2);
    assert_eq!(all["facets"]["tts"], 2);
    let second = get(
        &c,
        &format!("{base}/providers?sort=name&page_size=3&page=2"),
    )
    .await;
    let keys: std::collections::HashSet<_> = all["items"]
        .as_array()
        .unwrap()
        .iter()
        .chain(second["items"].as_array().unwrap())
        .map(|r| r["key"].as_str().unwrap())
        .collect();
    assert_eq!(keys.len(), 5);
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
    for suffix in ["test/vad", "test/asr", "test/llm", "test/tts"] {
        let r = c
            .post(format!("{base}/providers/config:llm:primary/{suffix}"))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::CONFLICT);
        assert!(
            r.text()
                .await
                .unwrap()
                .contains("config_provider_operation_unsupported")
        );
    }
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
        Err(voice_agent_server::providers::VadError::Failed(
            "fixture".into(),
        ))
    }
    fn adapter(&self) -> &'static str {
        "fixture"
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
        Err(voice_agent_server::providers::AsrError::Failed(
            "fixture".into(),
        ))
    }
}
