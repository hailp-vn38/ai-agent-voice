//! Real Chat/Responses adapters against local HTTP fixtures; no external API.
use super::super::models::JobModels;
use super::*;
use crate::providers::llm::{
    BackgroundMode, BackgroundTurnResult, ConfiguredOpenAiLlm, LlmProvider, LlmRequest,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio_util::sync::CancellationToken;

struct NativePair {
    db: Arc<Database>,
    config: Arc<AppConfig>,
    root: ConfiguredOpenAiLlm,
    child: ConfiguredOpenAiLlm,
}
#[async_trait::async_trait]
impl JobModels for NativePair {
    async fn turn_traced(
        &self,
        _: &model::LlmRunContext,
        pin: &ProviderPin,
        request: LlmRequest,
        mode: BackgroundMode,
        _: i64,
        _: CancellationToken,
    ) -> Result<BackgroundTurnResult, JobError> {
        let resolved = self
            .db
            .background_resolve_provider(&self.config, pin)
            .await?;
        let provider = if pin.reference == "config:llm:planner" {
            assert_eq!(resolved.transport, "chat_completions");
            &self.root
        } else {
            assert_eq!(pin.reference, "config:llm:child");
            assert_eq!(resolved.transport, "responses");
            &self.child
        };
        provider
            .background_turn(request, mode)
            .await
            .map_err(|_| JobError::ModelFailed)
    }
}
#[tokio::test]
async fn native_root_chat_child_responses_roundtrips_keep_routes_and_history_isolated() {
    use axum::{Json, Router, routing::post};
    let roots = Arc::new(AtomicUsize::new(0));
    let children = Arc::new(AtomicUsize::new(0));
    let root_count = roots.clone();
    let child_count = children.clone();
    let app=Router::new().route("/chat/completions",post(move |Json(body):Json<Value>| {
        let counter=root_count.clone(); async move {
            assert_eq!(body["model"],"root-fixture");
            assert_eq!(body["parallel_tool_calls"],true);
            let round=counter.fetch_add(1,Ordering::SeqCst);
            let (id,name,args)=if round==0 {("root_delegate","subagent",json!({"agent":"research",
                "description":"Native qualification","prompt":"PRIVATE_CHILD_TASK: read JSON and report Ada",
                "model":"config:llm:child","sessionID":null,"background":false}))}
                else {
                    let messages=body["messages"].as_array().unwrap();
                    assert!(messages.iter().any(|m|m["role"]=="tool"&&m["tool_call_id"]=="root_delegate"));
                    let raw=body.to_string();
                    assert!(!raw.contains("child_native_call"));let observation=messages.iter().find(|m|m["role"]=="tool"&&m["tool_call_id"]=="root_delegate").unwrap()["content"].as_str().unwrap();
                    assert!(!observation.contains("PRIVATE_CHILD_TASK"));
                    assert!(!raw.contains("private-child-reasoning"));
                    ("root_finish","finish_job",json!({"summary":"Verified Ada","findings":["Ada"],"unresolved":[]}))
                };
            Json(json!({"choices":[{"finish_reason":"tool_calls","message":{"tool_calls":[{
                "id":id,"type":"function","function":{"name":name,"arguments":args.to_string()}}]}}]}))
        }
    })).route("/responses",post(move |Json(body):Json<Value>| {
        let counter=child_count.clone(); async move {
            assert_eq!(body["model"],"child-fixture"); assert_eq!(body["tool_choice"],"auto");
            let raw=body.to_string();assert!(raw.contains("PRIVATE_CHILD_TASK"));assert!(!raw.contains("root_delegate"));
            let output=if counter.fetch_add(1,Ordering::SeqCst)==0 {
                json!([{"type":"reasoning","summary":"private-child-reasoning"},
                    {"type":"function_call","call_id":"child_native_call","name":"p_core_data_json_query",
                        "arguments":"{\"json_text\":\"{\\\"name\\\":\\\"Ada\\\"}\",\"pointer\":\"/name\"}"}])
            } else {
                assert!(body["input"].as_array().unwrap().iter().any(|m|m["type"]=="function_call_output"&&m["call_id"]=="child_native_call"));
                json!([{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Ada"}]}])
            };
            Json(json!({"status":"completed","output":output}))
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let mut config = config();
    let crate::config::LlmInstanceConfig::Openai(root) =
        config.providers.llm.instances.get_mut("planner").unwrap()
    else {
        panic!()
    };
    root.base_url = base.parse().unwrap();
    root.model = "root-fixture".into();
    let mut child = root.clone();
    child.model = "child-fixture".into();
    child.supports_responses = true;
    config.providers.llm.instances.insert(
        "child".into(),
        crate::config::LlmInstanceConfig::Openai(child),
    );
    let config = Arc::new(config);
    let db = Arc::new(Database::connect(&config.database).await.unwrap());
    let models = Arc::new(NativePair {
        db: db.clone(),
        config: config.clone(),
        root: ConfiguredOpenAiLlm::build_with_responses("fixture", &base, "root-fixture", 5, false)
            .unwrap()
            .with_background_capabilities(true, true),
        child: ConfiguredOpenAiLlm::build_with_responses(
            "fixture",
            &base,
            "child-fixture",
            5,
            true,
        )
        .unwrap()
        .with_background_capabilities(true, true),
    });
    let runtime = BackgroundRuntime::execution_harness(db.clone(), config, models)
        .await
        .unwrap();
    let revision = runtime.engine.settings().await.unwrap().revision;
    runtime
        .engine
        .update_settings(
            BackgroundSettingsUpdate {
                enabled: true,
                orchestrator_llm_provider: Some("config:llm:planner".into()),
                worker_llm_provider: Some("config:llm:child".into()),
            },
            revision,
        )
        .await
        .unwrap();
    let job = db
        .submit_job(
            &runtime.engine.config,
            request("Local native qualification"),
            &JobRequester::Admin,
            crate::database::jobs::SubmissionIdentity {
                ..Default::default()
            },
            crate::database::unix_seconds().unwrap(),
            || true,
        )
        .await
        .unwrap()
        .job;
    runtime.engine.wake.notify_one();
    let job = wait_for(&runtime.engine, &job.id, |j| {
        matches!(j.status.as_str(), "succeeded" | "failed")
    })
    .await;
    assert_eq!(job.status, "succeeded", "{:?}", job.error_code);
    assert_eq!(roots.load(Ordering::SeqCst), 2);
    assert_eq!(children.load(Ordering::SeqCst), 2);
    assert_ne!(job.snapshot.orchestrator, job.snapshot.worker);
    runtime.shutdown().await;
    server.abort();
}
