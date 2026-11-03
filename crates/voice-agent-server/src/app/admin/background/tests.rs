use super::attachments::{
    JobAttachmentMediaKind, decode_attachment_base64, sanitize_job_attachment_name,
    sanitize_wav_attachment_name, valid_attachment_mime_for_kind, valid_mp4_mime, valid_wav_mime,
};

#[test]
fn wav_and_mp4_attachment_validation_sanitizes_name_mime_and_base64() {
    assert_eq!(
        sanitize_wav_attachment_name("C:\\fakepath\\cuoc-hop.WAV").as_deref(),
        Some("cuoc-hop.WAV")
    );
    assert_eq!(
        sanitize_wav_attachment_name("/tmp/recordings/voice_note.wav").as_deref(),
        Some("voice_note.wav")
    );
    assert_eq!(
        sanitize_job_attachment_name("/tmp/videos/demo-clip.MP4"),
        Some(("demo-clip.MP4".to_owned(), JobAttachmentMediaKind::Mp4))
    );
    assert!(sanitize_job_attachment_name("not-supported.mp3").is_none());
    assert!(sanitize_job_attachment_name("").is_none());

    assert!(valid_wav_mime("audio/wav"));
    assert!(valid_wav_mime("audio/x-wav"));
    assert!(valid_wav_mime("audio/wave"));
    assert!(!valid_wav_mime("audio/mpeg"));
    assert!(valid_mp4_mime("video/mp4"));
    assert!(!valid_mp4_mime("video/webm"));
    assert!(valid_attachment_mime_for_kind(
        "video/mp4",
        JobAttachmentMediaKind::Mp4
    ));
    assert!(!valid_attachment_mime_for_kind(
        "audio/wav",
        JobAttachmentMediaKind::Mp4
    ));

    let decoded = decode_attachment_base64("UklGRg==").expect("base64");
    assert_eq!(decoded, b"RIFF");
}

#[tokio::test]
async fn resume_waiting_api() {
    use crate::{
        app::{AppState, new_lifecycle, router_with_state},
        config::AppConfig,
        database::{Database, credentials::CredentialCipher, secrets::SecretResolver},
        providers::ProviderSet,
    };
    use async_trait::async_trait;
    use orchestrator_agent_rs::{
        AgentDefinition, AgentMode, CustomModelExecutor, GatewayError, LlmBackend, LlmConfig,
        LlmProviderConfig, ModelRequest, ModelResponse, Orchestrator, OrchestratorConfig,
        ProviderAuth, RequestLimits, StreamSink,
    };
    use orchestrator_flow_rs::{CreateTaskRequest, TaskConfig, TaskManager, TaskStatus};
    use std::{
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };
    use tokio_util::sync::CancellationToken;

    #[derive(Debug)]
    struct RateLimitOnceExecutor {
        calls: AtomicUsize,
    }

    #[async_trait]
    impl CustomModelExecutor for RateLimitOnceExecutor {
        fn routing_fingerprint(&self) -> String {
            "test-rate-limit-once".to_string()
        }

        async fn complete(
            &self,
            _request: &ModelRequest,
            _cancel: &CancellationToken,
            _sink: StreamSink<'_>,
        ) -> Result<ModelResponse, GatewayError> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                return Err(GatewayError::RateLimited {
                    retry_after_secs: Some(60),
                });
            }
            Ok(ModelResponse {
                text: "Resumed and completed.".to_string(),
                finish_reason: "stop".to_string(),
                tool_calls: Vec::new(),
                usage: None,
            })
        }
    }

    let temp_dir =
        std::env::temp_dir().join(format!("admin-jobs-resume-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let db_path = temp_dir.join("server.db");
    let prompt_path = temp_dir.join("daily_root.md");
    std::fs::write(&prompt_path, "You are a helpful test assistant.").unwrap();

    let config_str = format!(
        r#"
[server]
bind = "127.0.0.1:0"
public_ws_url = "ws://127.0.0.1:0/voice/v1/"
[database]
url = "sqlite://{}?mode=rwc"
[provider_defaults]
vad = "test"
asr = "test"
llm = "test"
tts = "test"
[api]
enabled = true
admin_token = "jobs-admin-token"
"#,
        db_path.display()
    );
    let config: AppConfig = toml::from_str(&config_str).unwrap();
    let database = Database::connect(&config.database).await.unwrap();
    let resolver: Arc<dyn SecretResolver> = Arc::new(CredentialCipher::new(1, &[9u8; 32]).unwrap());
    let lifecycle = new_lifecycle(&config);
    let mut state = AppState::from_provider_set_with_database_resolver_and_shutdown(
        config,
        Arc::new(ProviderSet::unavailable()),
        Some(database),
        resolver,
        lifecycle,
    );

    let mut limits = RequestLimits::uniform(Duration::from_secs(10));
    limits.fast_retries = 0;
    let llm_cfg = LlmConfig {
        providers: vec![LlmProviderConfig {
            id: "test".to_string(),
            backend: LlmBackend::OpenAi,
            endpoint: None,
            auth: ProviderAuth::CustomExecutor(Arc::new(RateLimitOnceExecutor {
                calls: AtomicUsize::new(0),
            })),
            limits,
        }],
    };
    let agent = AgentDefinition::new("daily_root", "test", "gpt-test")
        .with_mode(AgentMode::Primary)
        .with_description("Daily root")
        .with_prompt_path(&prompt_path);

    let orch_cfg = OrchestratorConfig::new(llm_cfg, vec![agent], temp_dir.join("orch_store"));
    let orch = Arc::new(Orchestrator::builder(orch_cfg).build().await.unwrap());

    let task_cfg =
        TaskConfig::new(temp_dir.join("task_store")).with_default_target("daily", "daily_root");
    let task_mgr = Arc::new(
        TaskManager::prepare(task_cfg)
            .await
            .unwrap()
            .register_flow("daily", orch)
            .start()
            .await
            .unwrap(),
    );
    state.task_manager = Some(Arc::clone(&task_mgr));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = router_with_state(state);
    let server_handle = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let base = format!("http://{addr}/api/admin");
    let client = reqwest::Client::new();

    let created = task_mgr
        .create_task(CreateTaskRequest::new("Test waiting", "Run task"))
        .await
        .unwrap();

    // Wait until task transitions to Waiting
    for _ in 0..50 {
        let view = task_mgr.get_task_any_scope(&created.id).await.unwrap();
        if view.status == TaskStatus::Waiting {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let waiting_view = task_mgr.get_task_any_scope(&created.id).await.unwrap();
    assert_eq!(waiting_view.status, TaskStatus::Waiting);

    // POST /api/admin/jobs/{id}/resume on waiting task -> 200 OK
    let resume_resp = client
        .post(format!("{base}/jobs/{}/resume", created.id))
        .bearer_auth("jobs-admin-token")
        .send()
        .await
        .unwrap();
    assert_eq!(resume_resp.status(), reqwest::StatusCode::OK);
    let body: serde_json::Value = resume_resp.json().await.unwrap();
    assert!(
        matches!(body["status"].as_str(), Some("running" | "succeeded")),
        "unexpected status after resume: {:?}",
        body["status"]
    );

    // Wait until task reaches Completed
    for _ in 0..50 {
        let view = task_mgr.get_task_any_scope(&created.id).await.unwrap();
        if view.status == TaskStatus::Completed {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let done_view = task_mgr.get_task_any_scope(&created.id).await.unwrap();
    assert_eq!(done_view.status, TaskStatus::Completed);

    // POST /api/admin/jobs/{id}/resume on completed task -> 409 CONFLICT
    let conflict_resp = client
        .post(format!("{base}/jobs/{}/resume", created.id))
        .bearer_auth("jobs-admin-token")
        .send()
        .await
        .unwrap();
    assert_eq!(conflict_resp.status(), reqwest::StatusCode::CONFLICT);

    server_handle.abort();
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[tokio::test]
async fn recipe_admin_http_lists_validates_and_runs_playlist_recipe() {
    use crate::{
        app::{AppState, new_lifecycle, router_with_state},
        config::AppConfig,
        database::{Database, credentials::CredentialCipher, secrets::SecretResolver},
        providers::ProviderSet,
        services::task_flow::{PLAYLIST_RECIPE_ROOT, PlaylistSourcesRecipe},
    };
    use orchestrator_agent_rs::{
        AgentDefinition, AgentMode, HostTool, LlmConfig, Orchestrator, OrchestratorConfig,
    };
    use orchestrator_flow_rs::{TaskConfig, TaskManager, WorkspaceToolKit};
    use std::{sync::Arc, time::Duration};
    use tokio::sync::Notify;

    let temp = std::env::temp_dir().join(format!("admin-recipe-http-{}", uuid::Uuid::new_v4()));
    let vault = temp.join("vault");
    let prompt = temp.join("recipe.md");
    let ytdlp_calls = temp.join("ytdlp-calls.log");
    std::fs::create_dir_all(&temp).unwrap();
    std::fs::write(&prompt, "recipe root").unwrap();
    let fake_ytdlp = temp.join("yt-dlp");
    let fake_script = "#!/bin/sh\nif [ \"$1\" = \"--flat-playlist\" ]; then case \"$*\" in *fail*) exit 1;; *mixed*) printf 'bad\\tBad Video\\nvidmix\\tGood Video\\n';; *) printf 'vid123\\tSample Video\\n';; esac; exit 0; fi\nvideo=''\nfor arg in \"$@\"; do case \"$arg\" in *watch?v=*) video=\"${arg##*watch?v=}\";; esac; done\nprintf '%s\\n' \"$video\" >> 'CALLS_LOG_PATH'\ncase \"$video\" in bad) exit 1;; esac\nout=''\nwhile [ \"$#\" -gt 0 ]; do if [ \"$1\" = \"-o\" ]; then out=\"$2\"; shift 2; else shift; fi; done\ndir=\"${out%/*}\"\nmkdir -p \"$dir\"\ncat > \"$dir/$video.en-orig.vtt\" <<VTT\nWEBVTT\n\n00:00:00.000 --> 00:00:02.000\nA useful source transcript for $video.\nVTT\n";
    std::fs::write(
        &fake_ytdlp,
        fake_script.replace("CALLS_LOG_PATH", &ytdlp_calls.display().to_string()),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake_ytdlp, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let config: AppConfig = toml::from_str(&format!("[server]\nbind='127.0.0.1:0'\npublic_ws_url='ws://127.0.0.1:0/voice/v1/'\n[database]\nurl='sqlite://{}?mode=rwc'\n[provider_defaults]\nvad='test'\nasr='test'\nllm='test'\ntts='test'\n[api]\nenabled=true\nadmin_token='recipe-admin-token'\n", temp.join("server.db").display())).unwrap();
    let engine = Arc::new(llm_wiki::WikiEngine::open(&vault).unwrap());
    let init = TaskManager::prepare(
        TaskConfig::new(temp.join("tasks"))
            .with_wiki_vault_dir(&vault)
            .with_default_agent(PLAYLIST_RECIPE_ROOT),
    )
    .await
    .unwrap()
    .register_recipe(Arc::new(PlaylistSourcesRecipe))
    .unwrap();
    let runtime = init.recipe_provider_runtime().unwrap();
    let kit = WorkspaceToolKit::new(
        engine.clone(),
        Arc::new(Notify::new()),
        temp.join("reports"),
        temp.join("pdf"),
        temp.join("yt"),
    )
    .with_ytdlp(&fake_ytdlp, Duration::from_secs(5));
    assert!(
        init.validate_recipe_registry(
            &std::collections::HashSet::from(["yt_list_playlist".to_string()]),
            &std::collections::HashSet::new(),
        )
        .is_err()
    );
    let kit_tools = kit.all_host_tools(&init);
    let registered_tools = kit_tools
        .iter()
        .map(|tool| tool.descriptor().name)
        .collect::<std::collections::HashSet<_>>();
    init.validate_recipe_registry(&registered_tools, &std::collections::HashSet::new())
        .unwrap();
    let root = AgentDefinition::new(PLAYLIST_RECIPE_ROOT, "recipe", "playlist_sources@1")
        .with_mode(AgentMode::Primary)
        .with_prompt_path(prompt)
        .with_allowed_tools([
            "yt_list_playlist",
            "collection_status",
            "yt_fetch_transcript",
            "add_to_sources",
        ])
        .with_allowed_subagents(std::iter::empty::<String>())
        .with_steps(64)
        .with_context_limit_tokens(1_000_000);
    let mut builder = Orchestrator::builder(OrchestratorConfig::new(
        LlmConfig { providers: vec![] },
        vec![root],
        temp.join("orch"),
    ))
    .with_provider_runtime(runtime);
    for tool in kit_tools {
        builder = builder.register_tool(tool);
    }
    let orchestrator = Arc::new(builder.build().await.unwrap());
    let manager = init
        .with_orchestrator(orchestrator.clone())
        .start()
        .await
        .unwrap();

    let db = Database::connect(&config.database).await.unwrap();
    let resolver: Arc<dyn SecretResolver> = Arc::new(CredentialCipher::new(1, &[9u8; 32]).unwrap());
    let mut state = AppState::from_provider_set_with_database_resolver_and_shutdown(
        config.clone(),
        Arc::new(ProviderSet::unavailable()),
        Some(db),
        resolver,
        new_lifecycle(&config),
    );
    state.wiki_engine = Some(engine);
    state.orchestrator = Some(orchestrator);
    state.task_manager = Some(manager.clone());
    let router = router_with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/api/admin", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = reqwest::Client::new();

    let list = client
        .get(format!("{base}/background/recipes"))
        .bearer_auth("recipe-admin-token")
        .send()
        .await
        .unwrap();
    assert_eq!(list.status(), reqwest::StatusCode::OK);
    let listed_recipes: serde_json::Value = list.json().await.unwrap();
    assert_eq!(listed_recipes["items"][0]["id"], "playlist_sources");
    assert_eq!(
        listed_recipes["items"][0]["outline"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert_eq!(
        listed_recipes["items"][0]["outline"][1]["host_tools"][0],
        "yt_fetch_transcript"
    );

    let invalid = client.post(format!("{base}/jobs")).bearer_auth("recipe-admin-token")
        .json(&serde_json::json!({"recipe":{"id":"playlist_sources","params":{"url":"file:///private/list"}}})).send().await.unwrap();
    assert_eq!(invalid.status(), reqwest::StatusCode::BAD_REQUEST);
    assert_eq!(
        invalid.json::<serde_json::Value>().await.unwrap()["error"]["validation_path"],
        "params.url"
    );

    let created = client.post(format!("{base}/jobs")).bearer_auth("recipe-admin-token")
        .json(&serde_json::json!({"recipe":{"id":"playlist_sources","params":{"url":"https://www.youtube.com/playlist?list=PL1"}}})).send().await.unwrap();
    assert_eq!(created.status(), reqwest::StatusCode::CREATED);
    let job: serde_json::Value = created.json().await.unwrap();
    let id = job["id"].as_str().unwrap();
    for _ in 0..100 {
        if manager
            .get_task_any_scope(id)
            .await
            .unwrap()
            .status
            .is_terminal()
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let task = manager.get_task_any_scope(id).await.unwrap();
    assert_eq!(
        task.status,
        orchestrator_flow_rs::TaskStatus::Completed,
        "{task:?}"
    );
    let detail = manager.get_task_detail_any_scope(id).await.unwrap();
    let summary = detail.execution_summary.unwrap();
    assert_eq!(summary.total_input_tokens, 0);
    assert_eq!(summary.total_output_tokens, 0);
    let files: Vec<_> = std::fs::read_dir(vault.join("sources/youtube/playlist"))
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    assert_eq!(files.len(), 1);
    let source = std::fs::read_to_string(files[0].path()).unwrap();
    assert!(source.contains("A useful source transcript for vid123."));
    assert!(source.contains("vid123"));

    let mixed = client
        .post(format!("{base}/jobs"))
        .bearer_auth("recipe-admin-token")
        .json(&serde_json::json!({"recipe":{"id":"playlist_sources","params":{"url":"https://www.youtube.com/playlist?list=mixed"}}}))
        .send()
        .await
        .unwrap();
    assert_eq!(mixed.status(), reqwest::StatusCode::CREATED);
    let mixed_job: serde_json::Value = mixed.json().await.unwrap();
    let mixed_id = mixed_job["id"].as_str().unwrap();
    for _ in 0..200 {
        if manager
            .get_task_any_scope(mixed_id)
            .await
            .unwrap()
            .status
            .is_terminal()
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(
        manager.get_task_any_scope(mixed_id).await.unwrap().status,
        orchestrator_flow_rs::TaskStatus::Completed
    );
    let bad_attempts = std::fs::read_to_string(&ytdlp_calls)
        .unwrap()
        .lines()
        .filter(|line| *line == "bad")
        .count();
    assert_eq!(
        bad_attempts, 6,
        "three bounded ledger attempts each try the two subtitle language variants"
    );
    let mixed_sources: Vec<_> = std::fs::read_dir(vault.join("sources/youtube/playlist"))
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    assert_eq!(
        mixed_sources.len(),
        2,
        "the failed sibling must not prevent the good video from being saved"
    );
    assert!(mixed_sources.iter().any(|file| {
        std::fs::read_to_string(file.path())
            .unwrap()
            .contains("vidmix")
    }));

    let failed = client
        .post(format!("{base}/jobs"))
        .bearer_auth("recipe-admin-token")
        .json(&serde_json::json!({"recipe":{"id":"playlist_sources","params":{"url":"https://www.youtube.com/playlist?list=fail"}}}))
        .send()
        .await
        .unwrap();
    assert_eq!(failed.status(), reqwest::StatusCode::CREATED);
    let failed_job: serde_json::Value = failed.json().await.unwrap();
    let failed_id = failed_job["id"].as_str().unwrap();
    for _ in 0..100 {
        if manager
            .get_task_any_scope(failed_id)
            .await
            .unwrap()
            .status
            .is_terminal()
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let failed_task = manager.get_task_any_scope(failed_id).await.unwrap();
    assert_eq!(failed_task.status, orchestrator_flow_rs::TaskStatus::Failed);
    assert_eq!(
        failed_task.error_code.as_deref(),
        Some("RECIPE_STEP_FAILED")
    );
    server.abort();
    let _ = std::fs::remove_dir_all(&temp);
}
