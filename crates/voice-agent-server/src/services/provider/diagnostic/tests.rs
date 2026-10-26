use std::{collections::HashMap, sync::Arc, time::Duration};

use tokio_util::sync::CancellationToken;

use super::*;
use crate::{
    providers::{
        DatabaseRuntimeState, LlmProvider,
        llm::{LlmError, LlmEventStream, LlmRequest},
    },
    workers::LlmRuntime,
};

struct TestLlm;

#[async_trait::async_trait]
impl LlmProvider for TestLlm {
    fn adapter(&self) -> &'static str {
        "test"
    }

    async fn stream(&self, _: LlmRequest) -> Result<LlmEventStream, LlmError> {
        Err(LlmError::Failed)
    }
}

fn service(desired_revision: i64) -> ProviderDiagnosticService {
    let registry = RuntimeCatalog {
        llm: HashMap::from([(
            "llm-a".into(),
            Arc::new(LlmRuntime::new(
                Arc::new(TestLlm),
                2,
                Duration::from_secs(1),
            )),
        )]),
        ..RuntimeCatalog::default()
    };
    let snapshot = DatabaseRuntimeSnapshot::from_states([(
        "llm-a".into(),
        DatabaseRuntimeState {
            provider_id: 42,
            desired_revision,
            status: DatabaseRuntimeStatus::Loaded,
            failure: None,
        },
    )]);
    ProviderDiagnosticService::new(
        Arc::new(registry),
        Some(Arc::new(snapshot)),
        None,
        ProviderDiagnosticLimiter::new(1),
        Duration::from_secs(1),
    )
}

fn target(revision: i64) -> ProviderDiagnosticTarget {
    ProviderDiagnosticTarget {
        key: "llm-a".into(),
        provider_type: ProviderType::Llm,
        desired_revision: revision,
    }
}

#[test]
fn terminal_acknowledgement_releases_runtime_and_diagnostic_capacity() {
    let service = service(7);
    let lease = service.begin(target(7)).unwrap();
    assert_eq!(lease.metadata.tested_runtime, DatabaseRuntimeStatus::Loaded);
    lease.acknowledge_terminal();
    assert!(service.begin(target(7)).is_ok());
}

#[test]
fn quarantine_releases_limiter_but_keeps_exact_runtime_capacity_consumed() {
    let service = service(7);
    service.begin(target(7)).unwrap().quarantine();
    assert!(matches!(
        service.begin(target(7)),
        Err(ProviderDiagnosticError::Busy)
    ));
}

#[test]
fn stale_runtime_is_explicit_but_remains_testable() {
    let service = service(7);
    let lease = service.begin(target(8)).unwrap();
    assert!(!lease.metadata.runtime_matches_desired);
    assert!(lease.metadata.requires_restart);
}

struct TimeoutOperation {
    events: Arc<std::sync::Mutex<Vec<&'static str>>>,
    terminal_acknowledges: bool,
}

#[async_trait::async_trait]
impl ProviderDiagnosticOperation for TimeoutOperation {
    type Output = ();

    async fn execute(
        &mut self,
        _: CancellationToken,
    ) -> Result<Self::Output, ProviderDiagnosticOperationError> {
        std::future::pending().await
    }

    fn cancel_exact(&mut self) {
        self.events.lock().unwrap().push("cancel");
    }

    async fn await_terminal_acknowledgement(&mut self) -> bool {
        self.events.lock().unwrap().push("ack");
        self.terminal_acknowledges
    }

    fn quarantine_exact(&mut self) {
        self.events.lock().unwrap().push("quarantine");
    }
}

#[tokio::test]
async fn timeout_cancels_then_acknowledges_before_releasing_capacity() {
    let service = ProviderDiagnosticService::new(
        Arc::new(RuntimeCatalog {
            llm: HashMap::from([(
                "llm-a".into(),
                Arc::new(LlmRuntime::new(
                    Arc::new(TestLlm),
                    2,
                    Duration::from_secs(1),
                )),
            )]),
            ..RuntimeCatalog::default()
        }),
        Some(Arc::new(DatabaseRuntimeSnapshot::from_states([(
            "llm-a".into(),
            DatabaseRuntimeState {
                provider_id: 42,
                desired_revision: 7,
                status: DatabaseRuntimeStatus::Loaded,
                failure: None,
            },
        )]))),
        None,
        ProviderDiagnosticLimiter::new(1),
        Duration::from_millis(1),
    );
    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    assert!(matches!(
        service
            .execute(
                target(7),
                TimeoutOperation {
                    events: Arc::clone(&events),
                    terminal_acknowledges: true
                },
                Duration::from_secs(1)
            )
            .await,
        Err(ProviderDiagnosticError::Timeout)
    ));
    assert_eq!(*events.lock().unwrap(), ["cancel", "ack"]);
    assert!(service.begin(target(7)).is_ok());
}

#[tokio::test]
async fn timeout_quarantines_the_exact_operation_when_acknowledgement_is_missing() {
    let service = service(7);
    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    assert!(matches!(
        service
            .execute(
                target(7),
                TimeoutOperation {
                    events: Arc::clone(&events),
                    terminal_acknowledges: false,
                },
                Duration::from_secs(1),
            )
            .await,
        Err(ProviderDiagnosticError::Timeout)
    ));
    assert_eq!(*events.lock().unwrap(), ["cancel", "ack", "quarantine"]);
    assert!(matches!(
        service.begin(target(7)),
        Err(ProviderDiagnosticError::Busy)
    ));
}

struct CancelAckOperation {
    started: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
    cancelled: Arc<tokio::sync::Notify>,
}
#[async_trait::async_trait]
impl ProviderDiagnosticOperation for CancelAckOperation {
    type Output = ();
    async fn execute(
        &mut self,
        _: CancellationToken,
    ) -> Result<(), ProviderDiagnosticOperationError> {
        self.started.notify_one();
        std::future::pending().await
    }
    fn cancel_exact(&mut self) {
        self.cancelled.notify_one();
    }
    async fn await_terminal_acknowledgement(&mut self) -> bool {
        self.release.notified().await;
        true
    }
    fn quarantine_exact(&mut self) {}
}
#[tokio::test]
async fn cancelled_waiter_keeps_diagnostic_admission_until_terminal_acknowledgement() {
    let service = Arc::new(service(7));
    let started = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let cancelled = Arc::new(tokio::sync::Notify::new());
    let operation = CancelAckOperation {
        started: started.clone(),
        release: release.clone(),
        cancelled: cancelled.clone(),
    };
    let owned = service.clone();
    let pending = tokio::spawn(async move {
        owned
            .execute(target(7), operation, Duration::from_secs(1))
            .await
    });
    started.notified().await;
    pending.abort();
    tokio::time::timeout(Duration::from_secs(1), cancelled.notified())
        .await
        .expect("cancel exact operation after waiter leaves");
    assert!(matches!(
        service.begin(target(7)),
        Err(ProviderDiagnosticError::Busy)
    ));
    release.notify_one();
    tokio::task::yield_now().await;
    assert!(service.begin(target(7)).is_ok());
}

#[tokio::test]
async fn llm_rejection_does_not_leave_provider_test_busy() {
    use crate::providers::llm::ConfiguredOpenAiLlm;
    use axum::{Router, http::StatusCode, routing::post};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route(
                "/v1/chat/completions",
                post(|| async { StatusCode::TOO_MANY_REQUESTS }),
            ),
        )
        .await
        .unwrap();
    });
    let mut service = service(7);
    service.registry = Arc::new(RuntimeCatalog {
        llm: HashMap::from([(
            "llm-a".into(),
            Arc::new(LlmRuntime::new(
                Arc::new(
                    ConfiguredOpenAiLlm::build("public", &base, "space-bunny-free", 1).unwrap(),
                ),
                2,
                Duration::from_secs(1),
            )),
        )]),
        ..RuntimeCatalog::default()
    });
    for _ in 0..2 {
        let operation = service
            .registry
            .llm_diagnostic("llm-a", LlmRequest::text_turn("hello".into()), 1024)
            .unwrap();
        assert_eq!(
            service
                .execute(target(7), operation, Duration::from_millis(100))
                .await
                .err(),
            Some(ProviderDiagnosticError::Failed)
        );
    }
    server.abort();
}

#[tokio::test]
async fn non_chat_llm_capabilities_dispatch_matching_diagnostic_operations() {
    use crate::{
        config::{OpenAiConfig, SecretString},
        providers::llm::ConfiguredOpenAiLlm,
    };
    use axum::{Json, Router, routing::post};
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use url::Url;

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = Url::parse(&format!("http://{}/v1", listener.local_addr().unwrap())).unwrap();
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route(
                    "/v1/embeddings",
                    post(|| async {
                        Json(serde_json::json!({
                            "data": [{ "embedding": [0.25, -0.5, 0.75] }]
                        }))
                    }),
                )
                .route(
                    "/v1/images/generations",
                    post(|Json(body): Json<serde_json::Value>| async move {
                        if body.get("size").and_then(serde_json::Value::as_str) != Some("1024x1024")
                            || body.get("response_format").is_some()
                        {
                            return Json(serde_json::json!({
                                "error": {
                                    "message": "Yêu cầu chưa được xử lý. Vui lòng kiểm tra tham số hoặc liên hệ hỗ trợ.",
                                    "type": "upstream_error",
                                    "code": null
                                }
                            }));
                        }
                        Json(serde_json::json!({
                            "data": [{
                                "url": format!("data:image/png;base64,{}", STANDARD.encode(b"PNG_DATA"))
                            }]
                        }))
                    }),
                )
                .route(
                    "/v1/videos",
                    post(|Json(body): Json<serde_json::Value>| async move {
                        assert_eq!(
                            body.get("size").and_then(serde_json::Value::as_str),
                            Some("1280x720")
                        );
                        assert_eq!(
                            body.get("seconds").and_then(serde_json::Value::as_u64),
                            Some(5)
                        );
                        Json(serde_json::json!({
                            "status": "completed",
                            "url": format!("data:video/mp4;base64,{}", STANDARD.encode(b"MP4_DATA"))
                        }))
                    }),
                ),
        )
        .await
        .unwrap();
    });

    let emb_provider = ConfiguredOpenAiLlm::from_config(&OpenAiConfig {
        api_key: SecretString::new("sk-emb"),
        base_url: base.clone(),
        model: "text-embedding-3-small".into(),
        timeout_ms: 2_000,
        tags: Some(vec!["embedding".into()]),
        ..OpenAiConfig::default()
    })
    .unwrap();

    let img_provider = ConfiguredOpenAiLlm::from_config(&OpenAiConfig {
        api_key: SecretString::new("sk-img"),
        base_url: base.clone(),
        model: "gemini-3-pro-image".into(),
        timeout_ms: 2_000,
        tags: Some(vec!["image".into()]),
        ..OpenAiConfig::default()
    })
    .unwrap();

    let vid_provider = ConfiguredOpenAiLlm::from_config(&OpenAiConfig {
        api_key: SecretString::new("sk-vid"),
        base_url: base,
        model: "seedance_2.0_fast".into(),
        timeout_ms: 2_000,
        tags: Some(vec!["video".into()]),
        ..OpenAiConfig::default()
    })
    .unwrap();

    let mut service = service(7);
    service.registry = Arc::new(RuntimeCatalog {
        llm: HashMap::from([(
            "llm-a".into(),
            Arc::new(LlmRuntime::new(
                Arc::new(emb_provider),
                2,
                Duration::from_secs(2),
            )),
        )]),
        ..RuntimeCatalog::default()
    });

    let emb_op = service
        .registry
        .llm_diagnostic("llm-a", LlmRequest::text_turn("hello vector".into()), 1024)
        .unwrap();
    let (emb_summary, _) = service
        .execute(target(7), emb_op, Duration::from_secs(1))
        .await
        .unwrap();
    assert_eq!(emb_summary, "Embedded 1 input(s) (dimensions=3)");

    service.registry = Arc::new(RuntimeCatalog {
        llm: HashMap::from([(
            "llm-a".into(),
            Arc::new(LlmRuntime::new(
                Arc::new(img_provider),
                2,
                Duration::from_secs(2),
            )),
        )]),
        ..RuntimeCatalog::default()
    });

    let img_op = service
        .registry
        .llm_diagnostic("llm-a", LlmRequest::text_turn("draw icon".into()), 1024)
        .unwrap();
    let img_media_slot = img_op.media_slot();
    let (img_summary, _) = service
        .execute(target(7), img_op, Duration::from_secs(1))
        .await
        .unwrap();
    assert_eq!(img_summary, "Generated image/png (8 bytes)");
    let img_media = img_media_slot
        .lock()
        .unwrap()
        .take()
        .expect("image media should be captured");
    assert_eq!(img_media.mime_type, "image/png");
    assert_eq!(
        img_media.data_url,
        format!("data:image/png;base64,{}", STANDARD.encode(b"PNG_DATA"))
    );

    service.registry = Arc::new(RuntimeCatalog {
        llm: HashMap::from([(
            "llm-a".into(),
            Arc::new(LlmRuntime::new(
                Arc::new(vid_provider),
                2,
                Duration::from_secs(2),
            )),
        )]),
        ..RuntimeCatalog::default()
    });

    let vid_op = service
        .registry
        .llm_diagnostic("llm-a", LlmRequest::text_turn("waves crashing".into()), 1024)
        .unwrap();
    let vid_media_slot = vid_op.media_slot();
    let (vid_summary, _) = service
        .execute(target(7), vid_op, Duration::from_secs(1))
        .await
        .unwrap();
    assert_eq!(vid_summary, "Generated video/mp4 (8 bytes)");
    let vid_media = vid_media_slot
        .lock()
        .unwrap()
        .take()
        .expect("video media should be captured");
    assert_eq!(vid_media.mime_type, "video/mp4");
    assert_eq!(
        vid_media.data_url,
        format!("data:video/mp4;base64,{}", STANDARD.encode(b"MP4_DATA"))
    );

    server.abort();
}
