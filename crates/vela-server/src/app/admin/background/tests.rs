#[path = "../../../../tests/support/admin_web_token.rs"]
mod admin_web_token;

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
jwt_secret = "jobs-test-jwt-signing-secret-0123456789"
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
    if let Some(database) = state.database.as_ref() {
        admin_web_token::seed_default_admin_password(database.pool()).await;
    }
    let router = router_with_state(state);
    let server_handle = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let base = format!("http://{addr}/api/admin");
    let web_token = admin_web_token::admin_web_token(&reqwest::Client::new(), base.trim_end_matches("/api/admin")).await;
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
        .bearer_auth(&web_token)
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
        .bearer_auth(&web_token)
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
        services::task_flow::{
            NormalizedWikiHostTool, PLAYLIST_COMPLETION_TOOL, PLAYLIST_DOMAIN_SELECTOR,
            PLAYLIST_RECIPE_ROOT, PlaylistSourcesRecipe, TopicSelectionCompletionTool,
        },
    };
    use orchestrator_agent_rs::{
        AgentDefinition, AgentMode, GatewayError, LlmConfig, ModelRequest, ModelResponse,
        Orchestrator, OrchestratorConfig, ProviderRuntime, RuntimeProvider, StreamSink,
        test_support::ScriptedGateway,
    };
    use orchestrator_flow_rs::{
        CreateTaskRequest, TaskConfig, TaskManager, TaskRecipeRequest, WorkspaceToolKit,
    };
    use std::{
        sync::{Arc, OnceLock},
        time::Duration,
    };
    use tokio::sync::Notify;

    struct RecipeAndGateway {
        recipes: Arc<orchestrator_flow_rs::RecipeProviderRuntime>,
        scripted: Arc<ScriptedGateway>,
    }

    #[async_trait::async_trait]
    impl ProviderRuntime for RecipeAndGateway {
        fn providers(&self) -> Vec<RuntimeProvider> {
            vec![
                RuntimeProvider::new("recipe", "test-recipe-runtime:v1"),
                RuntimeProvider::new("test", "scripted-test:v1"),
            ]
        }

        async fn complete(
            &self,
            request: &ModelRequest,
            cancel: &tokio_util::sync::CancellationToken,
            sink: StreamSink<'_>,
        ) -> Result<ModelResponse, GatewayError> {
            if request.provider_id == "recipe" {
                self.recipes.complete(request, cancel, sink).await
            } else {
                orchestrator_agent_rs::test_support::LlmGateway::complete(
                    self.scripted.as_ref(),
                    request,
                    cancel,
                    sink,
                )
                .await
            }
        }
    }

    let temp = std::env::temp_dir().join(format!("admin-recipe-http-{}", uuid::Uuid::new_v4()));
    let vault = temp.join("vault");
    let prompt = temp.join("recipe.md");
    let ytdlp_calls = temp.join("ytdlp-calls.log");
    std::fs::create_dir_all(&temp).unwrap();
    std::fs::write(&prompt, "recipe root").unwrap();
    let fake_ytdlp = temp.join("yt-dlp");
    let fake_script = "#!/bin/sh\nif [ \"$1\" = \"--flat-playlist\" ]; then case \"$*\" in *fail*) exit 1;; *mixed*) printf 'bad\\tBad Video\\nvidmix\\tGood Video\\n';; *max*) i=1; while [ $i -le 50 ]; do printf 'maxvid%02d\\tLong Playlist Video %02d\\n' $i $i; i=$((i + 1)); done;; *youtu.be/one*) printf 'vidone\\tVideo One\\n';; *youtu.be/two*) printf 'vidtwo\\tVideo Two\\n';; *) printf 'vid123\\tSample Video\\n';; esac; exit 0; fi\nvideo=''\nfor arg in \"$@\"; do case \"$arg\" in *watch?v=*) video=\"${arg##*watch?v=}\";; esac; done\nprintf '%s\\n' \"$video\" >> 'CALLS_LOG_PATH'\ncase \"$video\" in bad) exit 1;; esac\nout=''\nwhile [ \"$#\" -gt 0 ]; do if [ \"$1\" = \"-o\" ]; then out=\"$2\"; shift 2; else shift; fi; done\ndir=\"${out%/*}\"\nmkdir -p \"$dir\"\ncat > \"$dir/$video.en-orig.vtt\" <<VTT\nWEBVTT\n\n00:00:00.000 --> 00:00:02.000\nA useful source transcript for $video.\nVTT\n";
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

    let config: AppConfig = toml::from_str(&format!("[server]\nbind='127.0.0.1:0'\npublic_ws_url='ws://127.0.0.1:0/voice/v1/'\n[database]\nurl='sqlite://{}?mode=rwc'\n[provider_defaults]\nvad='test'\nasr='test'\nllm='test'\ntts='test'\n[api]\nenabled=true\nadmin_token='recipe-admin-token'\njwt_secret='recipe-test-jwt-signing-secret-0123456789'\n", temp.join("server.db").display())).unwrap();
    let engine = Arc::new(llm_wiki::WikiEngine::open(&vault).unwrap());
    let init = TaskManager::prepare(
        TaskConfig::new(temp.join("tasks"))
            .with_wiki_vault_dir(&vault)
            .with_default_agent(PLAYLIST_RECIPE_ROOT),
    )
    .await
    .unwrap()
    .register_recipe(Arc::new(PlaylistSourcesRecipe::default()))
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
    let registered_agents = std::collections::HashSet::from([PLAYLIST_DOMAIN_SELECTOR.to_owned()]);
    init.validate_recipe_registry(&registered_tools, &registered_agents)
        .unwrap();
    let root = AgentDefinition::new(PLAYLIST_RECIPE_ROOT, "recipe", "playlist_sources@1")
        .with_mode(AgentMode::Primary)
        .with_prompt_path(prompt.clone())
        .with_allowed_tools([
            "yt_list_playlist",
            "collection_status",
            "yt_fetch_transcript",
            "add_to_sources",
            "subagent",
        ])
        .with_allowed_subagents([PLAYLIST_DOMAIN_SELECTOR])
        .with_steps(64)
        .with_context_limit_tokens(1_000_000);
    let selector = AgentDefinition::new(PLAYLIST_DOMAIN_SELECTOR, "test", "gpt-test")
        .with_mode(AgentMode::Subagent)
        .with_prompt_path(&prompt)
        .with_allowed_tools([PLAYLIST_COMPLETION_TOOL])
        .with_steps(4);
    let gateway = ScriptedGateway::new();
    let mut builder = Orchestrator::builder(OrchestratorConfig::new(
        LlmConfig { providers: vec![] },
        vec![root, selector],
        temp.join("orch"),
    ))
    .with_provider_runtime(Arc::new(RecipeAndGateway {
        recipes: runtime,
        scripted: gateway.clone(),
    }))
    .register_tool(Arc::new(TopicSelectionCompletionTool));
    let recipe_start_manager = Arc::new(OnceLock::new());
    for tool in kit_tools {
        builder = builder.register_tool(Arc::new(NormalizedWikiHostTool::new(
            tool,
            recipe_start_manager.clone(),
        )));
    }
    let orchestrator = Arc::new(builder.build().await.unwrap());
    let manager = init
        .with_orchestrator(orchestrator.clone())
        .start()
        .await
        .unwrap();
    assert!(recipe_start_manager.set(manager.clone()).is_ok());

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
    if let Some(database) = state.database.as_ref() {
        admin_web_token::seed_default_admin_password(database.pool()).await;
    }
    let router = router_with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/api/admin", listener.local_addr().unwrap());
    let web_token = admin_web_token::admin_web_token(&reqwest::Client::new(), base.trim_end_matches("/api/admin")).await;
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = reqwest::Client::new();

    let list = client
        .get(format!("{base}/background/recipes"))
        .bearer_auth(&web_token)
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
        4
    );
    assert_eq!(listed_recipes["items"][0]["outline"][2]["kind"], "agent");

    let enqueue_topic_selection = || {
        gateway.push_tool_calls(
            "",
            vec![(
                "topic-selection",
                PLAYLIST_COMPLETION_TOOL,
                serde_json::json!({"domain":"youtube", "topic":"playlist"}),
            )],
        );
        gateway.push_text_with_usage(r#"{"domain":"youtube","topic":"playlist"}"#, 12, 4);
    };

    let invalid = client.post(format!("{base}/jobs")).bearer_auth(&web_token)
        .json(&serde_json::json!({"recipe":{"id":"playlist_sources","params":{"url":"file:///private/list"}}})).send().await.unwrap();
    assert_eq!(invalid.status(), reqwest::StatusCode::BAD_REQUEST);
    assert_eq!(
        invalid.json::<serde_json::Value>().await.unwrap()["error"]["validation_path"],
        "params.url"
    );

    enqueue_topic_selection();
    let created = client.post(format!("{base}/jobs")).bearer_auth(&web_token)
        .json(&serde_json::json!({"recipe":{"id":"playlist_sources","params":{"url":"https://www.youtube.com/playlist?list=PL1"}}})).send().await.unwrap();
    assert_eq!(created.status(), reqwest::StatusCode::CREATED);
    let job: serde_json::Value = created.json().await.unwrap();
    assert_eq!(job["title"], "Thu thập playlist · tối đa 50 video");
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
    assert!(summary.total_input_tokens > 0 || summary.total_output_tokens > 0);
    let files: Vec<_> = std::fs::read_dir(vault.join("sources/youtube/playlist"))
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    assert_eq!(files.len(), 1);
    let source = std::fs::read_to_string(files[0].path()).unwrap();
    assert!(source.contains("A useful source transcript for vid123."));
    assert!(source.contains("vid123"));
    assert_eq!(
        manager.deferred_wiki_ingest_video_ids().await.unwrap(),
        ["vid123"],
        "default false must durably defer Wiki Auto-Flow"
    );

    enqueue_topic_selection();
    let video_list = client
        .post(format!("{base}/jobs"))
        .bearer_auth(&web_token)
        .json(&serde_json::json!({
            "title": "Video URL batch",
            "recipe": {"id":"playlist_sources", "params": {
                "video_urls":["https://youtu.be/one", "https://youtu.be/two"],
                "ingest_to_wiki":true
            }}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(video_list.status(), reqwest::StatusCode::CREATED);
    let video_list_job: serde_json::Value = video_list.json().await.unwrap();
    assert_eq!(video_list_job["title"], "Video URL batch");
    let video_list_id = video_list_job["id"].as_str().unwrap();
    for _ in 0..200 {
        if manager
            .get_task_any_scope(video_list_id)
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
        manager
            .get_task_any_scope(video_list_id)
            .await
            .unwrap()
            .status,
        orchestrator_flow_rs::TaskStatus::Completed
    );
    let list_sources = std::fs::read_dir(vault.join("sources/youtube/playlist"))
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| std::fs::read_to_string(entry.path()).unwrap())
        .collect::<Vec<_>>();
    assert!(list_sources.iter().any(|source| source.contains("vidone")));
    assert!(list_sources.iter().any(|source| source.contains("vidtwo")));
    assert_eq!(
        manager.deferred_wiki_ingest_video_ids().await.unwrap(),
        ["vid123"],
        "ingest=true must leave listed videos eligible for Auto-Flow"
    );

    enqueue_topic_selection();
    let mixed = client
        .post(format!("{base}/jobs"))
        .bearer_auth(&web_token)
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
        4,
        "the failed sibling must not prevent the good video from being saved"
    );
    assert!(mixed_sources.iter().any(|file| {
        std::fs::read_to_string(file.path())
            .unwrap()
            .contains("vidmix")
    }));

    enqueue_topic_selection();
    let failed = client
        .post(format!("{base}/jobs"))
        .bearer_auth(&web_token)
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

    enqueue_topic_selection();
    let max_playlist_task = manager
        .create_task(CreateTaskRequest {
            recipe: Some(TaskRecipeRequest {
                id: "playlist_sources".into(),
                params: serde_json::json!({
                    "url":"https://www.youtube.com/playlist?list=max",
                    "max_videos":50
                }),
            }),
            ..CreateTaskRequest::new("", "")
        })
        .await
        .unwrap();
    assert_eq!(
        max_playlist_task.title,
        "Thu thập playlist · tối đa 50 video"
    );
    for _ in 0..300 {
        if manager
            .get_task(&max_playlist_task.id)
            .await
            .unwrap()
            .status
            .is_terminal()
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let max_task = manager.get_task(&max_playlist_task.id).await.unwrap();
    assert_eq!(
        max_task.status,
        orchestrator_flow_rs::TaskStatus::Completed,
        "maximum playlist should complete without hitting the step cap: {max_task:?}"
    );
    let max_sources: Vec<_> = std::fs::read_dir(vault.join("sources/youtube/playlist"))
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    assert_eq!(max_sources.len(), 54);

    server.abort();
    let _ = std::fs::remove_dir_all(&temp);
}
