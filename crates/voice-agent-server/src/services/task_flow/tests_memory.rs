//! Background Task access to Memory, driven through real Tasks: a `VoiceTaskSubmit` creates the
//! Task, the Orchestrator runs it with a scripted model, and the test reads what the model got back.

use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use async_trait::async_trait;
use orchestrator_agent_rs::{
    AgentDefinition, AgentMode, HostTool, LlmBackend, LlmConfig, LlmProviderConfig, Orchestrator,
    OrchestratorConfig, ProviderAuth, RequestLimits, ToolContext, ToolDescriptor, ToolEffect,
    ToolError, ToolOutcome,
};
use orchestrator_flow_rs::{CreateTaskRequest, TaskConfig, TaskManager};
use serde_json::{Value, json};
use tokio::sync::Notify;

use super::{
    AgentConfigOverride, CUSTOM_DAILY_SUBAGENT_TOOLS, CUSTOM_WIKI_SUBAGENT_TOOLS,
    PersistedBackgroundSettings, TaskMemoryAccess, VoiceTaskSubmit, default_orchestrator_provider,
    init_orchestrator_and_task_manager, open_wiki_engine, project_agents_json,
};
use crate::{
    config::{DatabaseConfig, MemorySectionConfig, OrchestratorSectionConfig, WikiSectionConfig},
    database::{
        Database,
        devices::{DeviceInput, MemoryMode, get_device_by},
    },
    services::memory::{DeviceKind, FactKind, MemoryActor, MemoryChange, MemoryCore, MemoryScope},
};

fn temp_root(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("task-memory-{label}-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    root
}

/// The OpenAI client refuses to start without a key; the scripted server ignores it.
const SCRIPTED_API_KEY: &str = "scripted";

/// An OpenAI server on loopback. Calls the queued tools one per Task, then ends the turn
/// once the tool result is back. Everything past the HTTP request is the real thing.
#[derive(Debug, Default)]
struct ScriptedModel {
    base_url: String,
    calls: Mutex<VecDeque<(String, String, Value)>>,
    requests: Mutex<Vec<Value>>,
}

impl ScriptedModel {
    async fn serve() -> Arc<Self> {
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

    fn queue(&self, name: &str, arguments: Value) -> String {
        let id = format!("call_{}", uuid::Uuid::new_v4().simple());
        self.calls
            .lock()
            .unwrap()
            .push_back((id.clone(), name.to_owned(), arguments));
        id
    }

    fn tool_result(&self, call_id: &str) -> Option<String> {
        self.requests.lock().unwrap().iter().find_map(|request| {
            request["input"]
                .as_array()?
                .iter()
                .find(|item| item["type"] == "function_call_output" && item["call_id"] == call_id)
                .and_then(|item| item["output"].as_str())
                .map(str::to_owned)
        })
    }

    fn offered_tools(&self) -> Vec<String> {
        let requests = self.requests.lock().unwrap();
        let Some(tools) = requests
            .last()
            .and_then(|request| request["tools"].as_array())
        else {
            return Vec::new();
        };
        tools
            .iter()
            .filter_map(|tool| tool["name"].as_str().map(str::to_owned))
            .collect()
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
    let call = if tool_result_is_back {
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

fn scripted_provider(id: &str, model: &ScriptedModel) -> LlmProviderConfig {
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

struct OwnerProbe {
    seen: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl HostTool for OwnerProbe {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "owner_probe".to_owned(),
            description: "Records the owner the runtime gives a tool.".to_owned(),
            input_schema: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
            effect: ToolEffect::ReadOnly,
        }
    }

    async fn execute(&self, ctx: ToolContext, _args: Value) -> Result<ToolOutcome, ToolError> {
        self.seen.lock().unwrap().push(ctx.scope().owner_id.clone());
        Ok(ToolOutcome {
            call_id: ctx.scope().tool_call_id.clone(),
            content: "ok".to_owned(),
            metadata: Value::Null,
        })
    }
}

async fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    for _ in 0..200 {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("timed out waiting for {what}");
}

fn write_prompt(root: &Path) -> PathBuf {
    let path = root.join("daily_root.md");
    std::fs::write(&path, "You are a test assistant.").unwrap();
    path
}

/// Premise of ticket 06: Memory isolation for Tasks rests on this owner reaching the tool.
#[tokio::test]
async fn task_flow_voice_task_owner_reaches_daily_root_tool_scope() {
    let root = temp_root("premise");
    let model = ScriptedModel::serve().await;
    let seen = Arc::new(Mutex::new(Vec::new()));
    let agent = AgentDefinition::new("daily_root", "test", "test-model")
        .with_mode(AgentMode::Primary)
        .with_description("Daily root")
        .with_prompt_path(write_prompt(&root))
        .with_allowed_tools(["owner_probe"]);
    let llm = LlmConfig {
        providers: vec![scripted_provider("test", &model)],
    };
    let orchestrator = Arc::new(
        Orchestrator::builder(OrchestratorConfig::new(llm, vec![agent], root.join("orch")))
            .register_tool(Arc::new(OwnerProbe { seen: seen.clone() }))
            .build()
            .await
            .unwrap(),
    );
    let manager =
        TaskManager::prepare(TaskConfig::new(root.join("tasks")).with_default_agent("daily_root"))
            .await
            .unwrap()
            .with_orchestrator(orchestrator)
            .start()
            .await
            .unwrap();

    model.queue("owner_probe", json!({}));
    let submit = VoiceTaskSubmit::admitted(Some(&manager), 42, 7).unwrap();
    let created = submit
        .execute(
            "task_create",
            json!({ "title": "Premise", "prompt": "Check the owner" }),
            Duration::from_secs(5),
        )
        .await;
    assert_eq!(created["agent_id"], "daily_root", "task_create: {created}");

    wait_until("the probe to run", || !seen.lock().unwrap().is_empty()).await;
    assert_eq!(*seen.lock().unwrap(), vec!["device:42".to_owned()]);

    let _ = manager.shutdown().await;
    let _ = std::fs::remove_dir_all(&root);
}

/// The real task flow assembly, with its configured endpoint pointing at the scripted model.
struct TaskFlow {
    root: PathBuf,
    model: Arc<ScriptedModel>,
    manager: Arc<TaskManager>,
    database: Arc<Database>,
    core: Option<Arc<MemoryCore>>,
}

impl TaskFlow {
    async fn boot(memory_enabled: bool) -> Self {
        let root = temp_root("flow");
        let database = Arc::new(
            Database::connect(&DatabaseConfig {
                url: format!("sqlite://{}?mode=rwc", root.join("server.db").display()),
                max_connections: 4,
                busy_timeout_ms: 5_000,
                migrate_on_start: true,
                devices: Default::default(),
                history: Default::default(),
            })
            .await
            .unwrap(),
        );
        sqlx::query(
            "INSERT INTO agents (key, name, enabled, created_at, updated_at)
             VALUES ('home', 'Home', 1, 1, 1)",
        )
        .execute(database.pool())
        .await
        .unwrap();
        let core = memory_enabled.then(|| {
            Arc::new(
                MemoryCore::open(&MemorySectionConfig {
                    enabled: true,
                    vault_dir: root.join("vault"),
                    ..Default::default()
                })
                .unwrap(),
            )
        });

        let model = ScriptedModel::serve().await;
        let mut orchestrator = OrchestratorSectionConfig {
            enabled: true,
            endpoint: model.base_url.clone(),
            api_key: SCRIPTED_API_KEY.to_owned(),
            model: "test-model".to_owned(),
            store_dir: root.join("orch"),
            tasks_store_dir: root.join("tasks"),
            reports_dir: root.join("reports"),
            work_dir: root.join("work"),
            prompts_dir: root.join("prompts"),
            ..Default::default()
        };
        orchestrator.skills_dir = Some(root.join("skills"));
        let wiki = WikiSectionConfig {
            vault_dir: root.join("wiki"),
            ..Default::default()
        };
        let engine = open_wiki_engine(&wiki).unwrap();

        let (_, manager) = init_orchestrator_and_task_manager(
            &orchestrator,
            &wiki,
            engine,
            Arc::new(Notify::new()),
            &[default_orchestrator_provider(&orchestrator)],
            TaskMemoryAccess {
                core: core.clone(),
                database: Some(database.clone()),
            },
        )
        .await
        .unwrap();
        Self {
            root,
            model,
            manager,
            database,
            core,
        }
    }

    async fn device(&self, device_id: &str, mode: MemoryMode) -> i64 {
        self.database
            .create_device(
                DeviceInput {
                    device_id,
                    agent_key: "home",
                    template_key: None,
                    name: None,
                    description: None,
                    metadata: None,
                    memory_mode: mode,
                },
                "req-seed",
            )
            .await
            .map_err(|_| "create device failed")
            .unwrap();
        get_device_by(&self.database, device_id).await.unwrap().id
    }

    async fn remember(&self, device_id: &str, title: &str, body: &str) {
        self.core
            .as_ref()
            .unwrap()
            .apply(
                &MemoryScope::for_device(device_id, DeviceKind::Physical),
                MemoryActor::Admin,
                MemoryChange::Remember {
                    title: title.to_owned(),
                    hook: "ghi chú".to_owned(),
                    body: body.to_owned(),
                    kind: FactKind::Preference,
                    source_ref: None,
                },
            )
            .await
            .unwrap();
    }

    /// Runs one Task created through `VoiceTaskSubmit`, returns what the tool told the model.
    async fn voice_task_calls(&self, device_db_id: i64, tool: &str, args: Value) -> String {
        let call_id = self.model.queue(tool, args);
        let submit = VoiceTaskSubmit::admitted(Some(&self.manager), device_db_id, 7).unwrap();
        let created = submit
            .execute(
                "task_create",
                json!({ "title": "Read memory", "prompt": "Read memory" }),
                Duration::from_secs(5),
            )
            .await;
        assert_eq!(created["agent_id"], "daily_root", "task_create: {created}");
        self.tool_result(&call_id).await
    }

    async fn tool_result(&self, call_id: &str) -> String {
        for _ in 0..400 {
            if let Some(text) = self.model.tool_result(call_id) {
                return text;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!(
            "tool call never settled; tools offered: {:?}",
            self.model.offered_tools()
        );
    }

    async fn stop(self) {
        let _ = self.manager.shutdown().await;
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[tokio::test]
async fn task_flow_task_reads_memory_of_the_device_that_created_it() {
    let flow = TaskFlow::boot(true).await;
    let device_a = flow.device("device-a", MemoryMode::Read).await;
    let device_b = flow.device("device-b", MemoryMode::ReadWrite).await;
    flow.remember("device-a", "Cà phê của A", "A thích cà phê đen đá")
        .await;
    flow.remember("device-b", "Cà phê của B", "B thích cà phê sữa nóng")
        .await;

    let read_by_a = flow
        .voice_task_calls(device_a, "memory_recall", json!({ "query": "ca phe" }))
        .await;
    assert!(read_by_a.contains("A thích cà phê đen đá"), "{read_by_a}");
    assert!(!read_by_a.contains("B thích"), "{read_by_a}");
    let result: Value = serde_json::from_str(&read_by_a).unwrap();
    assert_eq!(result["untrusted_data"], true);

    let read_by_b = flow
        .voice_task_calls(device_b, "memory_recall", json!({ "query": "ca phe" }))
        .await;
    assert!(read_by_b.contains("B thích cà phê sữa nóng"), "{read_by_b}");
    assert!(!read_by_b.contains("A thích"), "{read_by_b}");

    let summary = flow
        .voice_task_calls(device_a, "memory_summary", json!({}))
        .await;
    assert!(summary.contains("Cà phê của A"), "{summary}");
    assert!(!summary.contains("Cà phê của B"), "{summary}");
    let result: Value = serde_json::from_str(&summary).unwrap();
    assert_eq!(result["untrusted_data"], true);

    flow.stop().await;
}

#[tokio::test]
async fn task_flow_task_without_a_live_device_owner_is_denied() {
    let flow = TaskFlow::boot(true).await;
    flow.device("device-a", MemoryMode::ReadWrite).await;
    flow.remember("device-a", "Cà phê của A", "A thích cà phê đen đá")
        .await;

    // Admin-created Tasks own `personal`, the default scope.
    let call_id = flow
        .model
        .queue("memory_recall", json!({ "query": "ca phe" }));
    flow.manager
        .create_task(CreateTaskRequest::new("Admin task", "Read memory"))
        .await
        .unwrap();
    let personal = flow.tool_result(&call_id).await;
    assert!(personal.contains("memory_scope_denied"), "{personal}");
    assert!(!personal.contains("đen đá"), "{personal}");

    let device_b = flow.device("device-b", MemoryMode::ReadWrite).await;
    sqlx::query("DELETE FROM devices WHERE id = ?")
        .bind(device_b)
        .execute(flow.database.pool())
        .await
        .unwrap();
    let deleted = flow
        .voice_task_calls(device_b, "memory_summary", json!({}))
        .await;
    assert!(deleted.contains("memory_scope_denied"), "{deleted}");

    flow.stop().await;
}

#[tokio::test]
async fn task_flow_device_with_memory_off_gets_memory_disabled() {
    let flow = TaskFlow::boot(true).await;
    let device = flow.device("device-off", MemoryMode::Off).await;
    flow.remember("device-off", "Cà phê", "thích cà phê đen đá")
        .await;

    let recall = flow
        .voice_task_calls(device, "memory_recall", json!({ "query": "ca phe" }))
        .await;
    assert!(recall.contains("memory_disabled"), "{recall}");
    assert!(!recall.contains("đen đá"), "{recall}");

    flow.stop().await;
}

#[tokio::test]
async fn task_flow_builds_with_memory_disabled_and_tools_report_it() {
    let flow = TaskFlow::boot(false).await;
    let device = flow.device("device-a", MemoryMode::ReadWrite).await;

    let recall = flow
        .voice_task_calls(device, "memory_recall", json!({ "query": "ca phe" }))
        .await;
    assert!(recall.contains("memory_disabled"), "{recall}");
    let summary = flow
        .voice_task_calls(device, "memory_summary", json!({}))
        .await;
    assert!(summary.contains("memory_disabled"), "{summary}");

    let offered = flow.model.offered_tools();
    assert!(offered.contains(&"memory_recall".to_owned()), "{offered:?}");
    assert!(
        offered.contains(&"memory_summary".to_owned()),
        "{offered:?}"
    );

    flow.stop().await;
}

#[test]
fn task_flow_only_daily_root_is_allowed_memory_tools() {
    let root = temp_root("allowlist");
    let cfg = OrchestratorSectionConfig {
        store_dir: root.join("store"),
        prompts_dir: root.join("prompts"),
        ..Default::default()
    };
    let catalog = vec![default_orchestrator_provider(&cfg)];
    let mut settings = PersistedBackgroundSettings::default();
    // An override on a built-in Sub-Agent cannot widen its tools.
    settings.agents.insert(
        "wiki_researcher".to_owned(),
        AgentConfigOverride {
            enabled: true,
            allowed_tools: Some(vec!["memory_recall".to_owned()]),
            ..Default::default()
        },
    );

    let agents = project_agents_json(&cfg, &settings, &catalog);
    let allowed =
        |id: &str| agents.iter().find(|agent| agent["id"] == id).unwrap()["allowed_tools"].clone();
    assert_eq!(
        allowed("daily_root"),
        json!(["subagent", "memory_recall", "memory_summary"])
    );
    assert_eq!(allowed("wiki_root"), json!(["subagent"]));
    for agent in agents.iter().filter(|agent| agent["id"] != "daily_root") {
        let tools = agent["allowed_tools"].as_array().unwrap();
        assert!(
            !tools.contains(&json!("memory_recall")) && !tools.contains(&json!("memory_summary")),
            "{} must not reach Memory",
            agent["id"]
        );
    }

    // Custom Sub-Agents pick from these pools.
    for tool in ["memory_recall", "memory_summary"] {
        assert!(!CUSTOM_DAILY_SUBAGENT_TOOLS.contains(&tool));
        assert!(!CUSTOM_WIKI_SUBAGENT_TOOLS.contains(&tool));
    }
    let _ = std::fs::remove_dir_all(&root);
}
