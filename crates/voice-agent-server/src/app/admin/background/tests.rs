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
