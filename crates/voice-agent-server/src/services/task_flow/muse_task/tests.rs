use super::MuseTaskTool;
use crate::providers::llm::{
    AgentFile, AgentTaskError, AgentTaskProvider, AgentTurnOutput, AgentTurnRequest,
    OperationJournal,
};
use async_trait::async_trait;
use orchestrator_agent_rs::{
    AgentDefinition, AgentMode, CallerContext, HostTool, LlmBackend, LlmConfig, LlmProviderConfig,
    Orchestrator, OrchestratorConfig, ProviderAuth, RequestLimits, ToolContext, ToolDescriptor,
    ToolError, ToolOutcome,
};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
pub(crate) fn temp_root(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("task-memory-{label}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    root
}

/// The OpenAI client refuses to start without a key; the scripted server ignores it.
const SCRIPTED_API_KEY: &str = "scripted";

/// An OpenAI server on loopback. Calls the queued tools one per Task, then ends the turn
/// once the tool result is back. Everything past the HTTP request is the real thing.
#[derive(Debug, Default)]
pub(crate) struct ScriptedModel {
    base_url: String,
    calls: Mutex<VecDeque<(String, String, Value)>>,
    requests: Mutex<Vec<Value>>,
    /// Keep calling queued tools after a tool result instead of ending the turn.
    chain_calls: std::sync::atomic::AtomicBool,
}

impl ScriptedModel {
    pub(crate) async fn serve() -> Arc<Self> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let model = Arc::new(Self {
            base_url: format!("http://{}/v1", listener.local_addr().unwrap()),
            ..Self::default()
        });
        let app = axum::Router::new()
            .fallback(respond)
            .with_state(model.clone());
        tokio::spawn(async move { axum::serve(listener, app).await });
        model
    }

    pub(crate) fn queue(&self, name: &str, arguments: Value) -> String {
        let id = format!("call_{}", uuid::Uuid::new_v4().simple());
        self.calls
            .lock()
            .unwrap()
            .push_back((id.clone(), name.to_owned(), arguments));
        id
    }

    pub(crate) fn tool_result(&self, call_id: &str) -> Option<String> {
        self.requests.lock().unwrap().iter().find_map(|request| {
            request["input"]
                .as_array()?
                .iter()
                .find(|item| item["type"] == "function_call_output" && item["call_id"] == call_id)
                .and_then(|item| item["output"].as_str())
                .map(str::to_owned)
        })
    }
}

/// The Responses API stream, which is what the OpenAI backend speaks.
async fn respond(
    axum::extract::State(model): axum::extract::State<Arc<ScriptedModel>>,
    axum::Json(request): axum::Json<Value>,
) -> impl axum::response::IntoResponse {
    let tool_result_is_back = request["input"]
        .as_array()
        .and_then(|input| input.last())
        .is_some_and(|item| item["type"] == "function_call_output");
    model.requests.lock().unwrap().push(request);
    let chained = model.chain_calls.load(std::sync::atomic::Ordering::SeqCst);
    let call = if tool_result_is_back && !chained {
        None
    } else {
        model.calls.lock().unwrap().pop_front()
    };
    let events = match call {
        Some((call_id, name, arguments)) => {
            let item =
                json!({ "id": "fc_1", "type": "function_call", "call_id": call_id, "name": name });
            vec![
                json!({ "type": "response.output_item.added", "output_index": 0, "item": item }),
                json!({
                    "type": "response.function_call_arguments.done",
                    "item_id": "fc_1",
                    "output_index": 0,
                    "arguments": arguments.to_string(),
                }),
                json!({ "type": "response.output_item.done", "output_index": 0, "item": item }),
            ]
        }
        None => vec![json!({ "type": "response.output_text.delta", "delta": "done" })],
    };
    let stream: String = events
        .iter()
        .chain([&json!({ "type": "response.completed", "response": {} })])
        .map(|event| format!("data: {event}\n\n"))
        .collect();
    ([(http::header::CONTENT_TYPE, "text/event-stream")], stream)
}

pub(crate) fn scripted_provider(id: &str, model: &ScriptedModel) -> LlmProviderConfig {
    let mut limits = RequestLimits::uniform(Duration::from_secs(10));
    limits.fast_retries = 0;
    LlmProviderConfig {
        id: id.to_owned(),
        backend: LlmBackend::OpenAiCompatible,
        endpoint: Some(model.base_url.clone()),
        auth: ProviderAuth::ApiKey(orchestrator_agent_rs::SecretString::new(SCRIPTED_API_KEY)),
        limits,
    }
}

#[derive(Default)]
struct FakeAgentTaskProvider {
    calls: Mutex<Vec<AgentTurnRequest>>,
    hold: std::sync::atomic::AtomicBool,
    output: Mutex<Option<Result<AgentTurnOutput, AgentTaskError>>>,
}
#[async_trait]
impl AgentTaskProvider for FakeAgentTaskProvider {
    async fn run_turn(
        &self,
        request: &AgentTurnRequest,
        _journal: &dyn OperationJournal,
        _progress: &(dyn for<'a> Fn(&'a str) + Send + Sync),
    ) -> Result<AgentTurnOutput, AgentTaskError> {
        self.calls.lock().unwrap().push(request.clone());
        if self.hold.load(std::sync::atomic::Ordering::SeqCst) {
            std::future::pending::<()>().await;
        }
        self.output.lock().unwrap().take().unwrap_or_else(|| {
            Ok(AgentTurnOutput {
                turn: 1,
                message: "Báo cáo".into(),
                files: vec![],
                files_missing: false,
            })
        })
    }
}
async fn run_tool(args: Value, child: bool) -> (String, Arc<FakeAgentTaskProvider>) {
    run_tool_output(args, child, None).await
}
async fn run_tool_output(
    args: Value,
    child: bool,
    output: Option<Result<AgentTurnOutput, AgentTaskError>>,
) -> (String, Arc<FakeAgentTaskProvider>) {
    run_tool_mode(args, child, output, false).await
}
async fn run_tool_mode(
    args: Value,
    child: bool,
    output: Option<Result<AgentTurnOutput, AgentTaskError>>,
    replay: bool,
) -> (String, Arc<FakeAgentTaskProvider>) {
    run_tool_probe(args, child, output, replay, None, false, 128 * 1024 * 1024).await
}
async fn run_tool_probe(
    args: Value,
    child: bool,
    output: Option<Result<AgentTurnOutput, AgentTaskError>>,
    replay: bool,
    seed: Option<orchestrator_agent_rs::ToolOperationStatus>,
    cancel: bool,
    artifact_limit: u64,
) -> (String, Arc<FakeAgentTaskProvider>) {
    run_tool_lifecycle(
        args,
        child,
        output,
        replay,
        seed,
        cancel,
        artifact_limit,
        false,
        false,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn run_tool_lifecycle(
    args: Value,
    child: bool,
    output: Option<Result<AgentTurnOutput, AgentTaskError>>,
    replay: bool,
    seed: Option<orchestrator_agent_rs::ToolOperationStatus>,
    cancel: bool,
    artifact_limit: u64,
    shutdown: bool,
    deadline: bool,
) -> (String, Arc<FakeAgentTaskProvider>) {
    let root = temp_root("muse-host");
    let model = ScriptedModel::serve().await;
    let prompt = root.join("test.md");
    std::fs::write(&prompt, "Test assistant").unwrap();
    let provider = Arc::new(FakeAgentTaskProvider::default());
    let retry = matches!(&output, Some(Err(AgentTaskError::RetryLater(_))));
    *provider.output.lock().unwrap() = output;
    provider
        .hold
        .store(shutdown || deadline, std::sync::atomic::Ordering::SeqCst);
    let primary = AgentDefinition::new("root", "test", "test-model")
        .with_mode(AgentMode::Primary)
        .with_prompt_path(&prompt)
        .with_allowed_tools(["muse_task", "subagent"])
        .with_allowed_subagents(["cloud"]);
    let sub = AgentDefinition::new("cloud", "test", "test-model")
        .with_mode(AgentMode::Subagent)
        .with_description("Cloud worker")
        .with_prompt_path(&prompt)
        .with_allowed_tools(["muse_task"]);
    let config = OrchestratorConfig::new(
        LlmConfig {
            providers: vec![scripted_provider("test", &model)],
        },
        vec![primary, sub],
        root.join("orch"),
    );
    let observed = Arc::new(Mutex::new(None));
    let orch = Orchestrator::builder(config)
        .register_tool(Arc::new(ReplayProbe {
            tool: MuseTaskTool::for_provider(provider.clone(), artifact_limit, 24 * 1024),
            replay,
            seed,
            cancel,
            shutdown,
            deadline,
            observed: observed.clone(),
        }))
        .build()
        .await
        .unwrap();
    if child {
        model.queue(
            "subagent",
            json!({"agent":"cloud","description":"Research","prompt":"Research"}),
        );
    }
    let call = model.queue("muse_task", args);
    let caller = CallerContext::new("owner", "workspace");
    let session = orch.create_session(&caller, "root").await.unwrap();
    orch.submit(&caller, &session, "Research".into())
        .await
        .unwrap();
    if shutdown {
        tokio::time::timeout(Duration::from_secs(5), async {
            while provider.calls.lock().unwrap().is_empty() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        orch.shutdown().await.unwrap();
    }
    let output = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(out) = model.tool_result(&call) {
                break out;
            }
            if let Some(out) = observed.lock().unwrap().clone() {
                break out;
            }
            if retry
                && let Some(waiting) = orch.waiting_descendant(&caller, &session).await.unwrap()
            {
                assert_eq!(waiting.error_code.as_deref(), Some("PROVIDER_UNAVAILABLE"));
                assert!(
                    waiting
                        .error_message
                        .as_deref()
                        .is_some_and(|message| message.contains("still running"))
                );
                orch.resume_waiting(&caller, &waiting.session_id)
                    .await
                    .unwrap();
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    orch.shutdown().await.unwrap();
    let _ = std::fs::remove_dir_all(root);
    (output, provider)
}
#[tokio::test]
async fn muse_task_root_is_rejected_before_provider_io() {
    let (out, provider) = run_tool(json!({"prompt":"Research"}), false).await;
    assert!(out.contains("MUSE_TASK_SUBAGENT_ONLY"), "{out}");
    assert!(provider.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn muse_task_subagent_receives_markdown_and_scoped_journal() {
    let (out,provider)=run_tool(json!({"prompt":"Research","output":"markdown","filename":"../research.md","max_wait_secs":90}),true).await;
    let result: Value = serde_json::from_str(&out).expect(&out);
    assert_eq!(result["status"], "completed");
    assert_eq!(result["message"], "Báo cáo");
    assert_eq!(result["files"][0]["filename"], "research.md");
    assert_eq!(result["files"][0]["mime_type"], "text/markdown");
    assert_eq!(result["files"][0]["origin"], "message");
    assert_eq!(result["files"][0]["size_bytes"], 9);
    assert_eq!(result["artifact_ids"][0], result["files"][0]["artifact_id"]);
    let calls = provider.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert!(calls[0].session_key.starts_with("muse_task:"));
    let request: Value =
        serde_json::from_str(calls[0].operation.request_json.as_ref().unwrap()).unwrap();
    assert_eq!(request["owner_id"], "owner");
    assert_eq!(request["workspace_id"], "workspace");
    assert_ne!(request["session_id"], request["root_session_id"]);
    assert_eq!(request["delete_session_on_finish"], false);
    assert_eq!(
        calls[0].session_key,
        format!("muse_task:{}", request["session_id"].as_str().unwrap())
    );
    assert_eq!(
        calls[0].operation.deadline_at - calls[0].operation.created_at,
        90
    );
}

#[tokio::test]
async fn muse_task_publishes_muse_markdown_and_attachment_metadata() {
    let (out, _) = run_tool_output(
        json!({"prompt":"Research","output":"markdown"}),
        true,
        Some(Ok(AgentTurnOutput {
            turn: 2,
            message: "Summary".into(),
            files: vec![AgentFile {
                filename: "../../report.md".into(),
                mime_type: "text/markdown".into(),
                bytes: b"# Report".to_vec(),
                source_path: "/tasks/report.md".into(),
            }],
            files_missing: false,
        })),
    )
    .await;
    let result: Value = serde_json::from_str(&out).expect(&out);
    assert_eq!(result["files"].as_array().unwrap().len(), 1);
    assert_eq!(result["files"][0]["origin"], "muse_file");
    assert_eq!(result["files"][0]["filename"], "report.md");
    assert_eq!(result["files"][0]["size_bytes"], 8);
    assert_eq!(
        result["files"][0]["sha256"],
        "04e1d1467e73933d8841c0c22eca9710ee72d020f5d494b091d68d4d2efea89d"
    );
}

#[tokio::test]
async fn muse_task_points_delivered_sandbox_links_at_their_artifact() {
    let (out, _) = run_tool_output(
        json!({"prompt":"Research"}),
        true,
        Some(Ok(AgentTurnOutput {
            turn: 1,
            message: "Saved [report.md](sandbox://tasks/report.md)".into(),
            files: vec![AgentFile {
                filename: "report.md".into(),
                mime_type: "text/markdown".into(),
                bytes: b"# Report".to_vec(),
                source_path: "tasks/report.md".into(),
            }],
            files_missing: false,
        })),
    )
    .await;
    let result: Value = serde_json::from_str(&out).expect(&out);
    let id = result["files"][0]["artifact_id"].as_str().unwrap();
    assert_eq!(
        result["message"],
        format!("Saved [report.md](artifact:{id})")
    );
    assert_eq!(result["status"], "completed");
}

#[tokio::test]
async fn muse_task_reports_a_missing_linked_file_for_a_text_turn() {
    let (out, _) = run_tool_output(
        json!({"prompt":"Research"}),
        true,
        Some(Ok(AgentTurnOutput {
            turn: 1,
            message: "Saved [gone.md](sandbox://tasks/gone.md)".into(),
            files: vec![],
            files_missing: true,
        })),
    )
    .await;
    let result: Value = serde_json::from_str(&out).expect(&out);
    assert_eq!(result["status"], "partial");
    assert_eq!(result["files_missing"], true);
    assert!(result["warning"].is_string());
}

#[tokio::test]
async fn muse_task_large_message_is_an_artifact_with_a_bounded_preview() {
    let (out, _) = run_tool_output(
        json!({"prompt":"Research"}),
        true,
        Some(Ok(AgentTurnOutput {
            turn: 1,
            message: "長".repeat(30_000),
            files: vec![],
            files_missing: false,
        })),
    )
    .await;
    let result: Value = serde_json::from_str(&out).expect(&out);
    assert!(
        out.len() < 24 * 1024,
        "tool content must fit the model read budget"
    );
    assert_eq!(result["files"][0]["origin"], "message");
    assert_eq!(result["files"][0]["size_bytes"], 90_000);
    assert!(result["artifact_ids"][0].as_str().is_some());
}

/// Answers every turn with the same message, as Muse does when collecting a finished turn.
struct SameTurnProvider(String);
#[async_trait]
impl AgentTaskProvider for SameTurnProvider {
    async fn run_turn(
        &self,
        _request: &AgentTurnRequest,
        _journal: &dyn OperationJournal,
        _progress: &(dyn for<'a> Fn(&'a str) + Send + Sync),
    ) -> Result<AgentTurnOutput, AgentTaskError> {
        Ok(AgentTurnOutput {
            turn: 1,
            message: self.0.clone(),
            files: vec![],
            files_missing: false,
        })
    }
}

/// Runs `muse_task` twice in one subagent and returns both tool results.
async fn run_send_then_collect(send: Value, collect: Value) -> (Value, Value) {
    let root = temp_root("muse-collect");
    let model = ScriptedModel::serve().await;
    model
        .chain_calls
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let prompt = root.join("test.md");
    std::fs::write(&prompt, "Test assistant").unwrap();
    let primary = AgentDefinition::new("root", "test", "test-model")
        .with_mode(AgentMode::Primary)
        .with_prompt_path(&prompt)
        .with_allowed_tools(["subagent"])
        .with_allowed_subagents(["cloud"]);
    let sub = AgentDefinition::new("cloud", "test", "test-model")
        .with_mode(AgentMode::Subagent)
        .with_description("Cloud worker")
        .with_prompt_path(&prompt)
        .with_allowed_tools(["muse_task"]);
    let config = OrchestratorConfig::new(
        LlmConfig {
            providers: vec![scripted_provider("test", &model)],
        },
        vec![primary, sub],
        root.join("orch"),
    );
    let orch = Orchestrator::builder(config)
        .register_tool(Arc::new(MuseTaskTool::for_provider(
            Arc::new(SameTurnProvider("長".repeat(30_000))),
            128 * 1024 * 1024,
            24 * 1024,
        )))
        .build()
        .await
        .unwrap();
    model.queue(
        "subagent",
        json!({"agent":"cloud","description":"Research","prompt":"Research"}),
    );
    let send_call = model.queue("muse_task", send);
    let collect_call = model.queue("muse_task", collect);
    let caller = CallerContext::new("owner", "workspace");
    let session = orch.create_session(&caller, "root").await.unwrap();
    orch.submit(&caller, &session, "Research".into())
        .await
        .unwrap();
    let (sent, collected) = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let (Some(sent), Some(collected)) = (
                model.tool_result(&send_call),
                model.tool_result(&collect_call),
            ) {
                break (sent, collected);
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    orch.shutdown().await.unwrap();
    let _ = std::fs::remove_dir_all(root);
    (
        serde_json::from_str(&sent).expect(&sent),
        serde_json::from_str(&collected).expect(&collected),
    )
}

#[tokio::test]
async fn muse_task_truncated_message_tells_the_model_how_to_read_it() {
    let (sent, _) =
        run_send_then_collect(json!({"prompt":"Research"}), json!({"action":"collect"})).await;
    assert_eq!(sent["message_truncated"], true);
    assert_eq!(sent["message_artifact_id"], sent["artifact_ids"][0]);
    let preview = sent["message"].as_str().unwrap();
    assert!(preview.contains("artifact_read"), "{preview}");
    assert!(
        chrono::DateTime::parse_from_rfc3339(sent["completed_at"].as_str().unwrap()).is_ok(),
        "{sent}"
    );
}

#[tokio::test]
async fn muse_task_collect_of_a_delivered_turn_reuses_its_artifacts() {
    let (sent, collected) =
        run_send_then_collect(json!({"prompt":"Research"}), json!({"action":"collect"})).await;
    assert_eq!(sent["artifact_ids"], collected["artifact_ids"]);
    assert_eq!(sent["completed_at"], collected["completed_at"]);
}

#[tokio::test]
async fn muse_task_collect_with_another_output_publishes_that_output() {
    let (sent, collected) = run_send_then_collect(
        json!({"prompt":"Research"}),
        json!({"action":"collect","output":"markdown"}),
    )
    .await;
    assert_ne!(sent["artifact_ids"], collected["artifact_ids"]);
    assert_eq!(collected["files"][0]["filename"], "report.md");
}

#[tokio::test]
async fn muse_task_large_markdown_keeps_one_message_artifact() {
    let (sent, _) = run_send_then_collect(
        json!({"prompt":"Research","output":"markdown"}),
        json!({"action":"collect","output":"markdown"}),
    )
    .await;
    assert_eq!(sent["artifact_ids"].as_array().unwrap().len(), 1, "{sent}");
    assert_eq!(sent["files"][0]["filename"], "report.md");
    assert_eq!(sent["message_artifact_id"], sent["artifact_ids"][0]);
}

#[tokio::test]
async fn muse_task_preserves_provider_failure_codes_and_partial_text() {
    for (failure, code) in [
        (AgentTaskError::Busy, "MUSE_TASK_BUSY"),
        (AgentTaskError::NoSession, "MUSE_TASK_NO_SESSION"),
        (AgentTaskError::SessionLost, "MUSE_SESSION_LOST"),
        (AgentTaskError::NeedsReauth, "PROVIDER_AUTH_FAILED"),
        (AgentTaskError::OutcomeUnknown, "TOOL_OUTCOME_UNKNOWN"),
        (
            AgentTaskError::Timeout {
                partial: Some("draft recovered".into()),
            },
            "MUSE_TASK_TIMEOUT",
        ),
        (
            AgentTaskError::Stalled {
                partial: Some("draft recovered".into()),
            },
            "MUSE_TASK_STALLED",
        ),
    ] {
        let (out, _) = run_tool_output(json!({"action":"collect"}), true, Some(Err(failure))).await;
        assert!(out.contains(code), "expected {code}: {out}");
        if code == "MUSE_TASK_TIMEOUT" || code == "MUSE_TASK_STALLED" {
            assert!(out.contains("draft recovered"));
            assert!(out.contains("collect"));
        }
    }
}
#[tokio::test]
async fn muse_task_invalid_arguments_never_reach_provider() {
    for args in [
        json!({"prompt":""}),
        json!({"action":"close"}),
        json!({"prompt":"x","max_wait_secs":59}),
        json!({"prompt":"x","max_wait_secs":7201}),
        json!({"prompt":"x","max_wait_secs":60.5}),
        json!({"prompt":"x","output":"exe"}),
        json!({"prompt":"x","delete_session_on_finish":"yes"}),
        json!({"prompt":"x","conversation_id":"other"}),
    ] {
        let (out, provider) = run_tool(args, true).await;
        assert!(out.contains("INVALID_ARGUMENT"), "{out}");
        assert!(provider.calls.lock().unwrap().is_empty());
    }
}

/// Calls the public HostTool boundary twice with the runtime-minted scope. All durable
/// operation reads/writes and artifact validation still use the real runtime services.
struct ReplayProbe {
    tool: MuseTaskTool,
    replay: bool,
    seed: Option<orchestrator_agent_rs::ToolOperationStatus>,
    cancel: bool,
    shutdown: bool,
    deadline: bool,
    observed: Arc<Mutex<Option<String>>>,
}
#[async_trait]
impl HostTool for ReplayProbe {
    fn descriptor(&self) -> ToolDescriptor {
        self.tool.descriptor()
    }
    async fn execute(&self, ctx: ToolContext, args: Value) -> Result<ToolOutcome, ToolError> {
        if let Some(status) = self.seed.or(if self.deadline {
            Some(orchestrator_agent_rs::ToolOperationStatus::Running)
        } else {
            None
        }) {
            let handle = ctx.operation();
            handle
                .begin(
                    handle.now_secs() + if self.deadline { 1 } else { -1 },
                    "muse_task",
                )
                .await?;
            handle
                .advance(orchestrator_agent_rs::OperationUpdate {
                    status: Some(status),
                    continuation_key: Some(format!("muse_task:{}", ctx.scope().session_id)),
                    ..Default::default()
                })
                .await?;
        }
        if self.cancel {
            ctx.cancel_token().cancel();
        }
        let result = match self.tool.execute(ctx.clone(), args.clone()).await {
            Ok(result) => result,
            Err(failure) => {
                if self.replay {
                    let replay = self
                        .tool
                        .execute(ctx.clone(), args)
                        .await
                        .expect_err("failed invocation should replay its error");
                    assert_eq!(replay.code, failure.code);
                    assert_eq!(replay.message, failure.message);
                }
                if self.shutdown {
                    assert_eq!(failure.code, "TOOL_RETRY_LATER");
                    assert!(
                        ctx.operation()
                            .current()
                            .await
                            .unwrap()
                            .unwrap()
                            .status
                            .is_open()
                    );
                }
                if self.cancel {
                    assert_eq!(
                        ctx.operation().current().await.unwrap().unwrap().status,
                        orchestrator_agent_rs::ToolOperationStatus::Cancelled
                    );
                }
                if self.shutdown || failure.code != "TOOL_RETRY_LATER" {
                    *self.observed.lock().unwrap() =
                        Some(format!("error {}: {}", failure.code, failure.message));
                }
                return Err(failure);
            }
        };
        if self.replay {
            let replay = self.tool.execute(ctx, args).await?;
            assert_eq!(result.content, replay.content);
            assert_eq!(result.metadata, replay.metadata);
        }
        Ok(result)
    }
}
#[tokio::test]
async fn muse_task_committed_invocation_replays_without_provider_or_artifact_duplication() {
    let (out, provider) = run_tool_mode(
        json!({"prompt":"Research","output":"markdown"}),
        true,
        None,
        true,
    )
    .await;
    assert!(out.contains("completed"), "{out}");
    assert_eq!(provider.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn muse_task_retry_waits_and_reuses_invocation_and_absolute_deadline() {
    let (out, provider) = run_tool_output(
        json!({"prompt":"Research","max_wait_secs":90}),
        true,
        Some(Err(AgentTaskError::RetryLater("still running".into()))),
    )
    .await;
    assert!(out.contains("completed"), "{out}");
    let calls = provider.calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(
        calls[0].operation.invocation_id,
        calls[1].operation.invocation_id
    );
    assert_eq!(
        calls[0].operation.deadline_at,
        calls[1].operation.deadline_at
    );
    assert_eq!(calls[0].session_key, calls[1].session_key);
}

#[tokio::test]
async fn muse_task_terminal_operations_never_call_provider() {
    use orchestrator_agent_rs::ToolOperationStatus as S;
    for (status, code) in [
        (S::Expired, "MUSE_TASK_TIMEOUT"),
        (S::Failed, "MUSE_TASK_REJECTED"),
        (S::Cancelled, "CANCELLED"),
        (S::Running, "MUSE_TASK_TIMEOUT"),
    ] {
        let (out, provider) = run_tool_probe(
            json!({"prompt":"Research"}),
            true,
            None,
            false,
            Some(status),
            false,
            1024,
        )
        .await;
        assert!(out.contains(code), "{out}");
        assert!(provider.calls.lock().unwrap().is_empty());
    }
}
#[tokio::test]
async fn muse_task_filters_disallowed_extensions_and_rejects_oversized_files() {
    let file = |name: &str| AgentTurnOutput {
        turn: 1,
        message: "Summary".into(),
        files: vec![AgentFile {
            filename: name.into(),
            mime_type: "application/octet-stream".into(),
            bytes: b"12345".to_vec(),
            source_path: "/tasks/a".into(),
        }],
        files_missing: false,
    };
    let (out, _) = run_tool_probe(
        json!({"prompt":"Research","output":"files"}),
        true,
        Some(Ok(file("payload.sh"))),
        false,
        None,
        false,
        4,
    )
    .await;
    let result: Value = serde_json::from_str(&out).expect(&out);
    assert_eq!(result["status"], "partial");
    assert!(result["files_missing"].as_bool().unwrap());
    assert!(result["artifact_ids"].as_array().unwrap().is_empty());
    let (out, _) = run_tool_probe(
        json!({"prompt":"Research"}),
        true,
        Some(Ok(file("large.pdf"))),
        false,
        None,
        false,
        4,
    )
    .await;
    assert!(out.contains("MUSE_TASK_FILE_TOO_LARGE"), "{out}");
}

#[tokio::test]
async fn muse_task_user_cancellation_rejects_before_provider_io() {
    let (out, provider) = run_tool_probe(
        json!({"prompt":"Research"}),
        true,
        None,
        false,
        None,
        true,
        1024,
    )
    .await;
    assert!(out.contains("CANCELLED"), "{out}");
    assert!(provider.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn muse_task_shutdown_keeps_submitted_operation_open_for_recovery() {
    let (out, provider) = run_tool_lifecycle(
        json!({"prompt":"Research"}),
        true,
        None,
        false,
        None,
        false,
        1024,
        true,
        false,
    )
    .await;
    assert!(out.contains("TOOL_RETRY_LATER"), "{out}");
    assert_eq!(provider.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn muse_task_failed_invocation_replays_original_timeout_partial_without_provider_io() {
    let (out, provider) = run_tool_mode(
        json!({"action":"collect"}),
        true,
        Some(Err(AgentTaskError::Timeout {
            partial: Some("recovered partial".into()),
        })),
        true,
    )
    .await;
    assert!(out.contains("MUSE_TASK_TIMEOUT"), "{out}");
    assert!(out.contains("recovered partial"));
    assert_eq!(provider.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn muse_task_json_escaped_message_respects_the_model_output_budget() {
    let (out, _) = run_tool_output(
        json!({"prompt":"Research"}),
        true,
        Some(Ok(AgentTurnOutput {
            turn: 1,
            message: "\0".repeat(10_000),
            files: vec![],
            files_missing: false,
        })),
    )
    .await;
    let result: Value = serde_json::from_str(&out).expect("valid JSON tool output");
    assert!(
        out.len() < 24 * 1024,
        "escaped message exceeded the output budget"
    );
    assert_eq!(result["files"][0]["size_bytes"], 10_000);
}

#[tokio::test]
async fn muse_task_absolute_deadline_stops_a_pending_provider_without_resending() {
    let (out, provider) = run_tool_lifecycle(
        json!({"prompt":"Research"}),
        true,
        None,
        false,
        None,
        false,
        1024,
        false,
        true,
    )
    .await;
    assert!(out.contains("MUSE_TASK_TIMEOUT"), "{out}");
    assert!(out.contains("collect"));
    assert_eq!(provider.calls.lock().unwrap().len(), 1);
}
