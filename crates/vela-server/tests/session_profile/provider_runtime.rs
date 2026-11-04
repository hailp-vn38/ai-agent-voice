use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::time::timeout;
use vela_server::{app::AppState, config::AppConfig, providers::ProviderSet};

use super::harness::*;

type ManagedBuild = (i64, i64, String, String);

struct ManagedFixtureBuilder {
    config: AppConfig,
    builds: Arc<Mutex<Vec<ManagedBuild>>>,
    supervisor: Arc<vela_server::workers::WorkerSupervisor>,
}

struct ManagedFixtureResource(vela_server::providers::RuntimeCatalog);

impl vela_server::services::provider_runtime::RuntimeResource for ManagedFixtureResource {
    fn unload(&self) -> bool {
        true
    }
    fn runtimes(&self) -> Option<vela_server::providers::RuntimeCatalog> {
        Some(self.0.clone())
    }
}

impl vela_server::services::provider_runtime::RuntimeMaterializer for ManagedFixtureBuilder {
    fn estimated_peak_bytes(
        &self,
        _: &vela_server::database::DesiredProvider,
    ) -> Result<u64, vela_server::services::provider_runtime::RuntimeError> {
        Ok(1)
    }
    fn logical_capacity(
        &self,
        _: &vela_server::database::DesiredProvider,
    ) -> Result<usize, vela_server::services::provider_runtime::RuntimeError> {
        Ok(8)
    }
    fn build(
        &self,
        row: &vela_server::database::DesiredProvider,
        _: Option<vela_server::services::provider_runtime::PreparedRuntime>,
        _: vela_server::workers::ProviderRuntimeAdmission,
    ) -> Result<
        Arc<dyn vela_server::services::provider_runtime::RuntimeResource>,
        vela_server::services::provider_runtime::RuntimeError,
    > {
        self.builds
            .lock()
            .unwrap()
            .push((row.id, row.revision, row.kind.clone(), row.key.clone()));
        let state = AppState::from_provider_set(
            self.config.clone(),
            Arc::new(ProviderSet::with_all(
                Arc::new(SilentVad),
                Arc::new(FinalAsr),
                Arc::new(EchoLlm {
                    label: if row.revision == 1 {
                        "managed-v1"
                    } else {
                        "managed-v2"
                    },
                }),
                Arc::new(ShortTts),
            )),
        );
        let runtimes = state
            .runtimes
            .resolve(&state.config.provider_defaults.effective_bindings())
            .unwrap();
        self.supervisor.observe_asr(runtimes.asr.clone());
        self.supervisor.observe_vad(runtimes.vad.clone());
        let kind = match row.kind.as_str() {
            "vad" => vela_server::providers::DiagnosticRuntimeKind::Vad,
            "asr" => vela_server::providers::DiagnosticRuntimeKind::Asr,
            "llm" => vela_server::providers::DiagnosticRuntimeKind::Llm,
            "tts" => vela_server::providers::DiagnosticRuntimeKind::Tts,
            _ => unreachable!(),
        };
        Ok(Arc::new(ManagedFixtureResource(
            vela_server::providers::RuntimeCatalog::single_provider(
                kind,
                row.key.clone(),
                &runtimes,
            ),
        )))
    }
}

#[tokio::test]
async fn managed_template_acquires_deployment_runtimes_for_missing_slots() {
    use vela_server::{
        database::{AdmittedAssignment, AdmittedProviderBinding, DesiredProvider},
        services::provider_runtime::{ProviderRuntimeManager, RuntimeLimits},
        session::{ManagedSessionProfileInput, resolve_managed_session_profile},
    };

    let state = AppState::from_provider_set(
        config("127.0.0.1:0".parse().unwrap(), database_url()),
        Arc::new(ProviderSet::with_all(
            Arc::new(SilentVad),
            Arc::new(FinalAsr),
            Arc::new(EchoLlm {
                label: "default-llm",
            }),
            Arc::new(ShortTts),
        )),
    );
    let mut profile_config = state.config.as_ref().clone();
    profile_config.provider_defaults.vad = "server_vad".into();
    profile_config.provider_defaults.asr = "server_asr".into();
    profile_config.provider_defaults.llm = "openai_primary".into();
    profile_config.provider_defaults.tts = "server_tts".into();
    let builds = Arc::new(Mutex::new(Vec::new()));
    let manager = ProviderRuntimeManager::new(
        RuntimeLimits {
            max_parallel_loads: 1,
            max_pending_loads: 0,
            max_waiters: 8,
            max_resident_bytes: 8,
            max_resources: 8,
            max_version_entries: 8,
            admission_timeout_ms: 2000,
            failure_cooldown_ms: 10,
            idle_ttl_ms: 1000,
        },
        Arc::new(ManagedFixtureBuilder {
            config: profile_config.clone(),
            builds: builds.clone(),
            supervisor: state.worker_supervisor.clone(),
        }),
        state.admission_gate().clone(),
    )
    .unwrap();
    let provider = |id: i64, kind: &str, key: &str| DesiredProvider {
        id,
        key: key.into(),
        kind: kind.into(),
        adapter: "fixture".into(),
        revision: 1,
        config_json: "{}".into(),
        secret_ref: None,
    };
    let assignment = AdmittedAssignment {
        template_id: 1,
        template_key: "partial".into(),
        template_name: "Partial".into(),
        language: "vi-VN".into(),
        prompt: TEMPLATE_PROMPT_V1.into(),
        template_enabled: true,
        template_revision: 1,
        is_default: true,
        assignment_enabled: true,
        bindings: [("asr", "db_asr", 1), ("tts", "db_tts", 2)]
            .map(|(kind, key, id)| AdmittedProviderBinding {
                provider_type: kind.into(),
                provider_key: key.into(),
                provider_enabled: true,
                snapshot: Some(Arc::new(provider(id, kind, key))),
            })
            .to_vec(),
    };
    let deployment = [
        provider(0, "vad", "server_vad"),
        provider(0, "llm", "openai_primary"),
    ];
    let profile = resolve_managed_session_profile(ManagedSessionProfileInput {
        device_db_id: 2,
        template_override_id: None,
        agent_id: 1,
        agent_key: "home",
        assignments: &[assignment],
        config: &profile_config,
        manager: &manager,
        deployment_snapshots: &deployment,
    })
    .await
    .unwrap();
    assert_eq!(profile.providers.vad, "server_vad");
    assert_eq!(profile.providers.asr, "db_asr");
    assert_eq!(profile.providers.llm, "openai_primary");
    assert_eq!(profile.providers.tts, "db_tts");
    assert!(profile.selected_runtimes.is_some());
    let actual = builds
        .lock()
        .unwrap()
        .iter()
        .map(|build| build.3.clone())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        actual,
        std::collections::BTreeSet::from([
            "server_vad".to_owned(),
            "db_asr".to_owned(),
            "openai_primary".to_owned(),
            "db_tts".to_owned(),
        ])
    );
}

#[tokio::test]
async fn public_api_created_provider_is_used_by_new_ws_and_patch_keeps_old_session_version() {
    use vela_server::services::provider_runtime::{ProviderRuntimeManager, RuntimeLimits};
    let url = database_url();
    let mut app_config = config("127.0.0.1:0".parse().unwrap(), url);
    app_config.api.enabled = true;
    app_config.api.admin_token = "managed-test-token".into();
    let builds = Arc::new(Mutex::new(Vec::new()));
    let recorded = builds.clone();
    let (base, url, task) = start_with_state(app_config, |state| {
        let manager = ProviderRuntimeManager::new(
            RuntimeLimits {
                max_parallel_loads: 1,
                max_pending_loads: 0,
                max_waiters: 32,
                max_resident_bytes: 16,
                max_resources: 16,
                max_version_entries: 32,
                admission_timeout_ms: 2000,
                failure_cooldown_ms: 10,
                idle_ttl_ms: 1000,
            },
            Arc::new(ManagedFixtureBuilder {
                config: state.config.as_ref().clone(),
                builds: recorded,
                supervisor: state.worker_supervisor.clone(),
            }),
            state.admission_gate().clone(),
        )
        .unwrap();
        state.with_runtime_manager(manager)
    })
    .await;
    let pool = seed(&url).await;
    sqlx::query("INSERT INTO devices (device_id,agent_id,enabled,created_at,updated_at) VALUES ('second',1,1,1,1)").execute(&pool).await.unwrap();
    let client = reqwest::Client::new();
    let auth = "managed-test-token";
    let providers = format!("{base}/api/admin/providers");
    let mut generated = Vec::new();
    for (kind, adapter, config_json) in [
        ("vad", "silero_onnx", serde_json::json!({})),
        (
            "asr",
            "gipformer_sherpa_offline",
            serde_json::json!({"decoding_method":"greedy_search","max_active_paths":4}),
        ),
        (
            "llm",
            "openai",
            serde_json::json!({"base_url":"https://example.test/v1","model":"version-one"}),
        ),
        ("tts", "zerotts_onnx", serde_json::json!({"voice":"maichi"})),
    ] {
        let response = client.post(&providers).bearer_auth(auth).json(&serde_json::json!({"name":kind,"type":kind,"adapter":adapter,"config_json":config_json})).send().await.unwrap();
        let status = response.status();
        let body: serde_json::Value = response.json().await.unwrap();
        assert_eq!(status, reqwest::StatusCode::CREATED, "{body}");
        generated.push((kind, body["key"].as_str().unwrap().to_owned()));
    }
    let llm_key = &generated[2].1;
    let response = client.post(format!("{base}/api/admin/templates")).bearer_auth(auth).json(&serde_json::json!({"key":"managed","name":"Managed","language":"vi-VN","prompt":"MANAGED-PROMPT"})).send().await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::CREATED);
    let mut template: serde_json::Value = response.json().await.unwrap();
    for (kind, provider_key) in &generated {
        let response = client
            .put(format!(
                "{base}/api/admin/templates/managed/providers/{kind}"
            ))
            .bearer_auth(auth)
            .header("if-match", format!("\"{}\"", template["revision"]))
            .json(&serde_json::json!({"provider_key":provider_key}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        template = response.json().await.unwrap();
    }
    let response = client
        .put(format!(
            "{base}/api/admin/agents/agent/default-template/managed"
        ))
        .bearer_auth(auth)
        .header("if-match", "\"1\"")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    // Binding the template enqueues a coalesced prewarm, which may already have run on a
    // loaded machine. It may only warm the bound revision-1 providers, each at most once.
    let prewarmed_only = |builds: &[ManagedBuild]| {
        builds.len() <= generated.len()
            && builds.iter().all(|(_, revision, kind, key)| {
                *revision == 1 && generated.iter().any(|(k, g)| k == kind && g == key)
            })
    };
    assert!(
        prewarmed_only(&builds.lock().unwrap()),
        "CRUD must only prewarm bound providers: {:?}",
        builds.lock().unwrap()
    );
    let cold: serde_json::Value = client
        .get(format!("{providers}/{llm_key}"))
        .bearer_auth(auth)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(cold["requires_restart"], false);
    assert_eq!(cold["runtime"]["desired_state"], "cold");
    assert!(prewarmed_only(&builds.lock().unwrap()));
    // With no pending-load slots, admitting during the prewarm batch would be refused as busy.
    let settle = tokio::time::Instant::now() + Duration::from_secs(10);
    while builds.lock().unwrap().len() < generated.len() && tokio::time::Instant::now() < settle {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(prewarmed_only(&builds.lock().unwrap()));
    let mut first = admit(&base).await;
    assert!(
        speak_and_observe(&mut first)
            .await
            .answer
            .starts_with("managed-v1[MANAGED-PROMPT]")
    );
    assert_eq!(builds.lock().unwrap().len(), 4);
    let response = client.patch(format!("{providers}/{llm_key}")).bearer_auth(auth).header("if-match", "\"1\"").json(&serde_json::json!({"config_json":{"base_url":"https://example.test/v1","model":"version-two"}})).send().await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let changed: serde_json::Value = client
        .get(format!("{providers}/{llm_key}"))
        .bearer_auth(auth)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(changed["runtime"]["desired_revision"], 2);
    assert_eq!(changed["runtime"]["desired_state"], "cold");
    assert_eq!(
        changed["runtime"]["ready_revisions"],
        serde_json::json!([1])
    );
    assert_eq!(builds.lock().unwrap().len(), 4);
    let mut second = admit_device(&base, "second", false).await;
    assert!(
        speak_and_observe(&mut second)
            .await
            .answer
            .starts_with("managed-v2[MANAGED-PROMPT]")
    );
    assert!(
        speak_and_observe(&mut first)
            .await
            .answer
            .starts_with("managed-v1[MANAGED-PROMPT]")
    );
    assert_eq!(
        builds.lock().unwrap().len(),
        5,
        "only changed exact version is built"
    );
    let created = client.post(&providers).bearer_auth(auth).json(&serde_json::json!({"name":"Unbound","type":"llm","adapter":"openai","config_json":{"base_url":"https://example.test/v1","model":"unbound"}})).send().await.unwrap();
    assert_eq!(created.status(), reqwest::StatusCode::CREATED);
    let created: serde_json::Value = created.json().await.unwrap();
    let unbound = created["key"].as_str().unwrap().to_owned();
    let prepared = client
        .post(format!("{providers}/{unbound}/prepare"))
        .bearer_auth(auth)
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap();
    assert!(
        [reqwest::StatusCode::OK, reqwest::StatusCode::ACCEPTED].contains(&prepared.status()),
        "{}",
        prepared.text().await.unwrap()
    );
    let prepared: serde_json::Value = prepared.json().await.unwrap();
    assert_eq!(prepared["provider_key"], unbound);
    assert_eq!(prepared["desired_revision"], 1);
    let invalid = client
        .post(format!("{providers}/{unbound}/prepare"))
        .bearer_auth(auth)
        .json(&serde_json::json!({"config_json":{"model":"override"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(invalid.status(), reqwest::StatusCode::BAD_REQUEST);
    timeout(Duration::from_secs(5), async {
        loop {
            let row: serde_json::Value = client
                .get(format!("{providers}/{unbound}"))
                .bearer_auth(auth)
                .send()
                .await
                .unwrap()
                .error_for_status()
                .unwrap()
                .json()
                .await
                .unwrap();
            if row["runtime"]["desired_state"] == "ready" {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the prepared provider becomes ready");
    let ready = client
        .post(format!("{providers}/{unbound}/prepare"))
        .bearer_auth(auth)
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(ready.status(), reqwest::StatusCode::OK);
    assert_eq!(
        builds.lock().unwrap().len(),
        6,
        "repeated prepare is idempotent"
    );
    let tested = client
        .post(format!("{providers}/{unbound}/test/llm"))
        .bearer_auth(auth)
        .json(&serde_json::json!({"input":"fixture"}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        tested.status(),
        reqwest::StatusCode::OK,
        "{}",
        tested.text().await.unwrap()
    );
    let diagnostic: serde_json::Value = tested.json().await.unwrap();
    assert_eq!(diagnostic["runtime"]["tested_revision"], 1);
    assert_eq!(diagnostic["runtime"]["requires_restart"], false);
    assert!(
        diagnostic["result"]["text"]
            .as_str()
            .unwrap()
            .starts_with("managed-v1")
    );
    assert_eq!(builds.lock().unwrap().len(), 6);
    let disabled = client
        .patch(format!("{providers}/{unbound}"))
        .bearer_auth(auth)
        .header("If-Match", "\"1\"")
        .json(&serde_json::json!({"enabled":false}))
        .send()
        .await
        .unwrap();
    assert_eq!(disabled.status(), reqwest::StatusCode::OK);
    let denied = client
        .post(format!("{providers}/{unbound}/prepare"))
        .bearer_auth(auth)
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), reqwest::StatusCode::CONFLICT);
    assert_eq!(builds.lock().unwrap().len(), 6);
    first.close(None).await.unwrap();
    second.close(None).await.unwrap();
    task.abort();
}

struct PrepareGateBuilder {
    inner: ManagedFixtureBuilder,
    entered: tokio::sync::mpsc::UnboundedSender<()>,
    release: Mutex<std::sync::mpsc::Receiver<()>>,
}

impl vela_server::services::provider_runtime::RuntimeMaterializer for PrepareGateBuilder {
    fn estimated_peak_bytes(
        &self,
        _: &vela_server::database::DesiredProvider,
    ) -> Result<u64, vela_server::services::provider_runtime::RuntimeError> {
        Ok(1)
    }
    fn logical_capacity(
        &self,
        _: &vela_server::database::DesiredProvider,
    ) -> Result<usize, vela_server::services::provider_runtime::RuntimeError> {
        Ok(2)
    }
    fn build(
        &self,
        row: &vela_server::database::DesiredProvider,
        prepared: Option<vela_server::services::provider_runtime::PreparedRuntime>,
        quota: vela_server::workers::ProviderRuntimeAdmission,
    ) -> Result<
        Arc<dyn vela_server::services::provider_runtime::RuntimeResource>,
        vela_server::services::provider_runtime::RuntimeError,
    > {
        self.entered.send(()).unwrap();
        self.release.lock().unwrap().recv().unwrap();
        self.inner.build(row, prepared, quota)
    }
}

#[tokio::test]
async fn prepare_public_api_returns_accepted_then_ready_and_rejects_loader_flood() {
    use vela_server::services::provider_runtime::{ProviderRuntimeManager, RuntimeLimits};
    let mut cfg = config("127.0.0.1:0".parse().unwrap(), database_url());
    cfg.api.enabled = true;
    cfg.api.admin_token = "prepare-test-token".into();
    let (entered, mut entered_rx) = tokio::sync::mpsc::unbounded_channel();
    let (release, release_rx) = std::sync::mpsc::channel();
    let (base, _, task) = start_with_state(cfg, |state| {
        let manager = ProviderRuntimeManager::new(
            RuntimeLimits {
                max_parallel_loads: 1,
                max_pending_loads: 0,
                max_waiters: 4,
                max_resident_bytes: 4,
                max_resources: 4,
                max_version_entries: 8,
                admission_timeout_ms: 2000,
                failure_cooldown_ms: 10,
                idle_ttl_ms: 1000,
            },
            Arc::new(PrepareGateBuilder {
                inner: ManagedFixtureBuilder {
                    config: state.config.as_ref().clone(),
                    builds: Arc::new(Mutex::new(vec![])),
                    supervisor: state.worker_supervisor.clone(),
                },
                entered,
                release: Mutex::new(release_rx),
            }),
            state.lifecycle.gate().clone(),
        )
        .unwrap();
        state.with_runtime_manager(manager)
    })
    .await;
    let client = reqwest::Client::new();
    let providers = format!("{base}/api/admin/providers");
    let mut generated = Vec::new();
    for name in ["first", "second"] {
        let created = client.post(&providers).bearer_auth("prepare-test-token")
            .json(&serde_json::json!({"name":name,"type":"llm","adapter":"openai","config_json":{"base_url":"https://example.test/v1","model":"fixture"}}))
            .send().await.unwrap();
        assert_eq!(created.status(), reqwest::StatusCode::CREATED);
        let created: serde_json::Value = created.json().await.unwrap();
        generated.push(created["key"].as_str().unwrap().to_owned());
    }
    let (first_key, second_key) = (&generated[0], &generated[1]);
    let first = client
        .post(format!("{providers}/{first_key}/prepare"))
        .bearer_auth("prepare-test-token")
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(first.status(), reqwest::StatusCode::ACCEPTED);
    let accepted: serde_json::Value = first.json().await.unwrap();
    assert_eq!(accepted["provider_key"], *first_key);
    assert_eq!(accepted["desired_revision"], 1);
    entered_rx.recv().await.unwrap();
    let blocked = client
        .post(format!("{providers}/{second_key}/prepare"))
        .bearer_auth("prepare-test-token")
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(blocked.status(), reqwest::StatusCode::TOO_MANY_REQUESTS);
    release.send(()).unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
    loop {
        let response = client
            .get(format!("{providers}/{first_key}"))
            .bearer_auth("prepare-test-token")
            .send()
            .await
            .unwrap();
        let row: serde_json::Value = response.json().await.unwrap();
        if row["runtime"]["desired_state"] == "ready" {
            break;
        }
        assert!(tokio::time::Instant::now() < deadline);
        tokio::task::yield_now().await;
    }
    let ready = client
        .post(format!("{providers}/{first_key}/prepare"))
        .bearer_auth("prepare-test-token")
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(ready.status(), reqwest::StatusCode::OK);
    task.abort();
}
