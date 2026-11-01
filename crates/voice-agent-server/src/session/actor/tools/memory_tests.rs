use super::*;
use crate::{
    config::{McpResultDelivery, MemorySectionConfig},
    database::{Database, devices::MemoryMode as StoredMode},
    services::memory::{
        DeviceKind, FactKind, MemoryActor, MemoryChange, MemoryCore, MemoryMode, MemoryScope,
        ReceiptAction,
    },
    session::SessionMemory,
    tools::{
        builtin::{EXIT_TOOL_NAME, MEMORY_RECALL_TOOL_NAME, MEMORY_TOOL_NAMES},
        system_mcp::tests::fixture,
    },
};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

const VIRTUAL_DEVICE: &str = "__system_default_device__";

struct Vaults {
    root: PathBuf,
    core: Arc<MemoryCore>,
}

impl Drop for Vaults {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

impl Vaults {
    fn open() -> Self {
        let root = std::env::temp_dir().join(format!("voice-memory-{}", uuid::Uuid::new_v4()));
        let core = MemoryCore::open(&MemorySectionConfig {
            enabled: true,
            vault_dir: root.clone(),
            ..MemorySectionConfig::default()
        })
        .unwrap();
        Self {
            root,
            core: Arc::new(core),
        }
    }

    async fn remember(&self, scope: &MemoryScope, title: &str, body: &str) -> (String, u64) {
        let receipt = self
            .core
            .apply(
                scope,
                MemoryActor::Admin,
                MemoryChange::Remember {
                    title: title.into(),
                    hook: format!("hook of {title}"),
                    body: body.into(),
                    kind: FactKind::Business,
                    source_ref: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(receipt.action, ReceiptAction::Remember);
        (
            receipt.fact_id.unwrap().as_str().to_owned(),
            receipt.revision.unwrap(),
        )
    }
}

fn physical_scope(device_id: &str) -> MemoryScope {
    MemoryScope::for_device(device_id, DeviceKind::Physical)
}

async fn database() -> Arc<Database> {
    let (_core, _context, db) = fixture(true, 4096).await;
    db
}

async fn add_device(db: &Database, device_id: &str, kind: &str, mode: StoredMode) -> i64 {
    let agent_id = crate::database::agents::get_agent_by(db, "studio")
        .await
        .unwrap()
        .id;
    let stored = match mode {
        StoredMode::Off => "off",
        StoredMode::Read => "read",
        StoredMode::ReadWrite => "read_write",
    };
    sqlx::query(
        "INSERT INTO devices (device_id, agent_id, kind, enabled, memory_mode, created_at, \
         updated_at) VALUES (?, ?, ?, 1, ?, 1000, 1000)",
    )
    .bind(device_id)
    .bind(agent_id)
    .bind(kind)
    .bind(stored)
    .execute(db.pool())
    .await
    .unwrap();
    db.device_row_id(device_id).await.unwrap().unwrap()
}

/// A session the way admission builds it, on a Device with a real `memory_mode`.
async fn admitted(
    vaults: &Vaults,
    db: &Database,
    device_id: &str,
    kind: &str,
    mode: StoredMode,
) -> Option<SessionMemory> {
    let row = add_device(db, device_id, kind, mode).await;
    SessionMemory::admit(Some(&vaults.core), Some(db), row).await
}

fn actor_with(memory: Option<SessionMemory>) -> SessionActor {
    let mut actor = tests::session_with_external_mcp(Default::default())
        .with_session_memory(memory)
        .with_tool_status(true);
    actor.mcp.result_delivery = McpResultDelivery::Silent;
    actor
}

/// Runs a conversational turn up to the LLM request and returns the System message it composed.
fn compose_next_turn(actor: &mut SessionActor) -> String {
    actor.turn = None;
    actor.begin_active_turn().unwrap();
    let text = "xin chào".to_owned();
    actor.commit_user_text(text.clone()).unwrap();
    actor.begin_speech_delivery(text);
    match actor.llm_messages.first() {
        Some(ChatMessage::System { content }) => content.clone(),
        other => panic!("expected the System message first, got {other:?}"),
    }
}

fn begin_tool_turn(actor: &mut SessionActor) {
    actor.begin_active_turn().unwrap();
    actor.commit_user_text("tìm giúp tôi".into()).unwrap();
}

fn tool_call(id: &str, name: &str, arguments: serde_json::Value) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: name.into(),
        arguments,
    }
}

async fn run_batch(actor: &mut SessionActor, calls: Vec<ToolCall>) {
    actor.start_tool_batch(calls);
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while actor.tool_batch.is_some() {
            actor.drain_async_call_completions();
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

/// The tool result envelope of one call, as the model receives it.
fn tool_result(actor: &SessionActor, call_id: &str) -> serde_json::Value {
    actor
        .llm_messages
        .iter()
        .find_map(|message| match message {
            ChatMessage::ToolResult {
                tool_call_id,
                content,
            } if tool_call_id == call_id => Some(serde_json::from_str(content).unwrap()),
            _ => None,
        })
        .expect("the call has a result")
}

fn recall_payload(actor: &SessionActor, call_id: &str) -> serde_json::Value {
    let envelope = tool_result(actor, call_id);
    assert_eq!(envelope["ok"], true, "{envelope}");
    serde_json::from_str(envelope["content"].as_str().unwrap()).unwrap()
}

async fn recall(actor: &mut SessionActor, arguments: serde_json::Value) -> serde_json::Value {
    begin_tool_turn(actor);
    run_batch(
        actor,
        vec![tool_call("recall", MEMORY_RECALL_TOOL_NAME, arguments)],
    )
    .await;
    recall_payload(actor, "recall")
}

fn memory_tool_count(actor: &SessionActor) -> usize {
    actor
        .available_llm_tools()
        .iter()
        .filter(|tool| tool.name.starts_with("server_memory_"))
        .count()
}

#[tokio::test]
async fn off_device_has_no_tool_and_no_memory_block() {
    let vaults = Vaults::open();
    let db = database().await;
    let scope = physical_scope("device-off");
    vaults.remember(&scope, "Giờ mở cửa", "Mở lúc 7 giờ").await;

    let memory = admitted(&vaults, &db, "device-off", "physical", StoredMode::Off).await;
    assert!(memory.is_none());
    let mut actor = actor_with(memory);

    assert_eq!(memory_tool_count(&actor), 0);
    assert!(!actor.offers_answer_changing_tools());
    let system = compose_next_turn(&mut actor);
    assert!(!system.contains("<memory_info>"));
    assert!(!system.contains("<memory_policy>"));
    assert!(!system.contains("Giờ mở cửa"));
}

#[tokio::test]
async fn a_hallucinated_memory_call_on_an_off_device_gets_memory_disabled() {
    let mut actor = actor_with(None);
    begin_tool_turn(&mut actor);
    run_batch(
        &mut actor,
        vec![tool_call(
            "recall",
            MEMORY_RECALL_TOOL_NAME,
            serde_json::json!({"query": "giờ"}),
        )],
    )
    .await;
    let result = tool_result(&actor, "recall");
    assert_eq!(result["ok"], false);
    assert_eq!(result["code"], "memory_disabled");
}

#[tokio::test]
async fn read_device_is_offered_recall_only_and_sees_its_facts() {
    let vaults = Vaults::open();
    let db = database().await;
    let scope = physical_scope("device-read");
    vaults.remember(&scope, "Giờ mở cửa", "Mở lúc 7 giờ").await;

    let memory = admitted(&vaults, &db, "device-read", "physical", StoredMode::Read)
        .await
        .unwrap();
    assert_eq!(memory.mode, MemoryMode::Read);
    let mut actor = actor_with(Some(memory));

    let offered: Vec<String> = actor
        .available_llm_tools()
        .into_iter()
        .map(|tool| tool.name)
        .filter(|name| name.starts_with("server_memory_"))
        .collect();
    assert_eq!(offered, ["server_memory_recall"]);
    assert!(actor.offers_answer_changing_tools());

    let system = compose_next_turn(&mut actor);
    assert!(system.contains("<memory_policy>"));
    assert!(system.contains("<memory_info>"));
    assert!(system.contains("Giờ mở cửa"));
}

#[tokio::test]
async fn memory_is_not_offered_when_admission_finds_no_usable_device() {
    let vaults = Vaults::open();
    let db = database().await;
    let row = add_device(&db, "device-a", "physical", StoredMode::Read).await;

    assert!(
        SessionMemory::admit(None, Some(&db), row).await.is_none(),
        "Memory disabled"
    );
    assert!(
        SessionMemory::admit(Some(&vaults.core), None, row)
            .await
            .is_none(),
        "no database"
    );
    assert!(
        SessionMemory::admit(Some(&vaults.core), Some(&db), 0)
            .await
            .is_none(),
        "no Device row"
    );
    assert!(
        SessionMemory::admit(Some(&vaults.core), Some(&db), row + 1000)
            .await
            .is_none(),
        "unknown Device"
    );
    db.pool().close().await;
    assert!(
        SessionMemory::admit(Some(&vaults.core), Some(&db), row)
            .await
            .is_none(),
        "a failed lookup still admits the session, without Memory"
    );
}

/// Without the delivery rule a Memory result would fall through to `Silent`, and the user would
/// hear nothing at all.
#[tokio::test]
async fn no_memory_tool_falls_through_to_silent_delivery() {
    let vaults = Vaults::open();
    let db = database().await;
    let memory = admitted(&vaults, &db, "device-a", "physical", StoredMode::Read).await;
    let actor = actor_with(memory);
    for name in MEMORY_TOOL_NAMES {
        let call = tool_call("call", name, serde_json::json!({}));
        assert_eq!(
            actor.round_result_delivery(std::slice::from_ref(&call)),
            McpResultDelivery::LlmThenTts,
            "{name}"
        );
        assert!(is_builtin_tool_name(name), "{name}");
        assert!(
            matches!(
                actor.resolve_tool(name),
                Some(ToolTarget::Builtin(
                    crate::tools::builtin::BuiltinTool::Memory
                ))
            ),
            "{name}"
        );
    }
}

#[tokio::test]
async fn a_goodbye_in_the_same_batch_does_not_skip_the_memory_result() {
    let vaults = Vaults::open();
    let db = database().await;
    let memory = admitted(&vaults, &db, "device-a", "physical", StoredMode::Read).await;
    let mut actor = actor_with(memory);
    begin_tool_turn(&mut actor);
    run_batch(
        &mut actor,
        vec![
            tool_call(
                "recall",
                MEMORY_RECALL_TOOL_NAME,
                serde_json::json!({"query": "giờ"}),
            ),
            tool_call(
                "exit",
                EXIT_TOOL_NAME,
                serde_json::json!({"say_goodbye": "Tạm biệt"}),
            ),
        ],
    )
    .await;
    // The round continues through the LLM, which is how the result gets spoken.
    assert_eq!(actor.tool_rounds.rounds, 1);
    assert!(actor.direct_request_id.is_none());
    assert_eq!(recall_payload(&actor, "recall")["untrusted_data"], true);
}

#[tokio::test]
async fn recall_returns_untrusted_items_with_id_and_revision() {
    let vaults = Vaults::open();
    let db = database().await;
    let scope = physical_scope("device-a");
    let (id, revision) = vaults
        .remember(&scope, "Đặt lịch", "Khách đặt lịch trước 2 giờ")
        .await;
    let memory = admitted(&vaults, &db, "device-a", "physical", StoredMode::Read).await;
    let mut actor = actor_with(memory);

    // ASR drops the diacritics, and the fact still matches.
    let payload = recall(&mut actor, serde_json::json!({"query": "dat lich"})).await;

    assert_eq!(payload["untrusted_data"], true);
    assert_eq!(payload["hidden"], 0);
    let item = &payload["items"][0];
    assert_eq!(item["id"], id);
    assert_eq!(item["revision"], revision);
    assert_eq!(item["title"], "Đặt lịch");
    assert_eq!(item["hook"], "hook of Đặt lịch");
    assert_eq!(item["body"], "Khách đặt lịch trước 2 giờ");
    assert_eq!(item["kind"], "business");
    assert!(item["updated_at"].is_string());
}

#[tokio::test]
async fn recall_caps_the_limit_at_five_and_cuts_the_body_at_500_chars() {
    let vaults = Vaults::open();
    let db = database().await;
    let scope = physical_scope("device-a");
    for index in 0..7 {
        let body = format!("nội dung {index} {}", "đ".repeat(600));
        vaults
            .remember(&scope, &format!("Fact {index}"), &body)
            .await;
    }
    let memory = admitted(&vaults, &db, "device-a", "physical", StoredMode::Read).await;
    let mut actor = actor_with(memory);

    let payload = recall(
        &mut actor,
        serde_json::json!({"query": "nội dung", "limit": 50}),
    )
    .await;

    let items = payload["items"].as_array().unwrap();
    assert_eq!(items.len(), 5);
    assert_eq!(payload["hidden"], 2);
    for item in items {
        assert_eq!(item["body"].as_str().unwrap().chars().count(), 500);
    }
}

#[tokio::test]
async fn recall_refuses_malformed_arguments_and_ignores_any_scope_from_the_model() {
    let vaults = Vaults::open();
    let db = database().await;
    vaults
        .remember(&physical_scope("device-b"), "Của B", "bí mật riêng")
        .await;
    let memory = admitted(&vaults, &db, "device-a", "physical", StoredMode::Read).await;
    let mut actor = actor_with(memory);
    begin_tool_turn(&mut actor);
    run_batch(
        &mut actor,
        vec![
            tool_call("missing", MEMORY_RECALL_TOOL_NAME, serde_json::json!({})),
            tool_call(
                "blank",
                MEMORY_RECALL_TOOL_NAME,
                serde_json::json!({"query": "  "}),
            ),
            tool_call(
                "zero",
                MEMORY_RECALL_TOOL_NAME,
                serde_json::json!({"query": "x", "limit": 0}),
            ),
            tool_call(
                "spoof",
                MEMORY_RECALL_TOOL_NAME,
                serde_json::json!({"query": "riêng", "device_id": "device-b", "scope": "device-b"}),
            ),
        ],
    )
    .await;
    for call in ["missing", "blank", "zero"] {
        assert_eq!(
            tool_result(&actor, call)["code"],
            "invalid_arguments",
            "{call}"
        );
    }
    assert_eq!(
        recall_payload(&actor, "spoof")["items"],
        serde_json::json!([])
    );
}

#[tokio::test]
async fn two_devices_on_one_agent_recall_only_their_own_facts() {
    let vaults = Vaults::open();
    let db = database().await;
    vaults
        .remember(&physical_scope("device-a"), "Của A", "chung của A")
        .await;
    vaults
        .remember(&physical_scope("device-b"), "Của B", "chung của B")
        .await;
    let memory = admitted(&vaults, &db, "device-a", "physical", StoredMode::Read).await;
    let mut actor = actor_with(memory);

    let payload = recall(&mut actor, serde_json::json!({"query": "chung"})).await;

    let items = payload["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["title"], "Của A");
    let system = compose_next_turn(&mut actor);
    assert!(system.contains("Của A"));
    assert!(!system.contains("Của B"));
}

#[tokio::test]
async fn admin_forget_disappears_at_the_next_turn_of_an_open_session() {
    let vaults = Vaults::open();
    let db = database().await;
    let scope = physical_scope("device-a");
    let (id, revision) = vaults.remember(&scope, "Sắp quên", "Nội dung").await;
    vaults.remember(&scope, "Còn lại", "Nội dung khác").await;
    let memory = admitted(&vaults, &db, "device-a", "physical", StoredMode::Read).await;
    let mut actor = actor_with(memory);
    assert!(compose_next_turn(&mut actor).contains("Sắp quên"));

    vaults
        .core
        .apply(
            &scope,
            MemoryActor::Admin,
            MemoryChange::Forget {
                id: crate::services::memory::FactId::parse(&id).unwrap(),
                expected_revision: revision,
            },
        )
        .await
        .unwrap();

    let system = compose_next_turn(&mut actor);
    assert!(!system.contains("Sắp quên"));
    assert!(system.contains("Còn lại"));
}

/// The Studio Assistant may hold a Bound Reminder Target on a physical Device, and that binding
/// must grant nothing here: Memory follows the session's own Virtual Device.
#[tokio::test]
async fn studio_assistant_uses_only_the_virtual_devices_vault_despite_a_bound_target() {
    let vaults = Vaults::open();
    let db = database().await;
    let physical_row = add_device(&db, "kitchen", "physical", StoredMode::Read).await;
    vaults
        .remember(&physical_scope("kitchen"), "Của bếp", "chung của bếp")
        .await;
    vaults
        .remember(
            &MemoryScope::for_device(VIRTUAL_DEVICE, DeviceKind::Virtual),
            "Của Studio",
            "chung của Studio",
        )
        .await;
    // The Virtual Device is a singleton that the database already holds.
    let virtual_device = db.assistant_device().await.unwrap().unwrap();
    sqlx::query("UPDATE devices SET memory_mode='read' WHERE id=?")
        .bind(virtual_device.id)
        .execute(db.pool())
        .await
        .unwrap();
    let memory = SessionMemory::admit(Some(&vaults.core), Some(&db), virtual_device.id).await;

    let notifications = crate::database::notifications::DeviceNotifications::new((*db).clone());
    notifications
        .set_grant("studio", true, 1, "grant-req-1")
        .await
        .unwrap();
    let reminders = crate::services::notifications::SessionReminderTools {
        coordinator: crate::services::notifications::ReminderCoordinator::new(
            notifications,
            Arc::new(crate::services::active_device_sessions::ActiveDeviceSessions::default()),
        ),
        agent_id: crate::database::agents::get_agent_by(&db, "studio")
            .await
            .unwrap()
            .id,
        agent_key: "studio".into(),
        default_timezone: "UTC".into(),
        target: crate::services::notifications::SessionReminderTarget::WebAssistant {
            assistant_device_id: VIRTUAL_DEVICE.into(),
            bound_target: Arc::new(std::sync::Mutex::new(Some(
                crate::services::notifications::BoundReminderTarget {
                    device_row_id: physical_row,
                    device_id: "kitchen".into(),
                    expires_at: tokio::time::Instant::now() + std::time::Duration::from_secs(600),
                },
            ))),
        },
    };
    let mut actor = actor_with(memory).with_reminder_tools(Some(reminders));

    let system = compose_next_turn(&mut actor);
    assert!(system.contains("Của Studio"));
    assert!(!system.contains("Của bếp"));
    assert!(system.contains("kitchen"), "the binding itself stays live");

    actor.turn = None;
    let payload = recall(&mut actor, serde_json::json!({"query": "chung"})).await;
    let items = payload["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["title"], "Của Studio");
}

#[derive(Clone, Default)]
struct CapturedLog(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for CapturedLog {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// ADR 0024. This runs on a current-thread runtime, so the actor and its spawned call both log
/// through the subscriber installed here.
#[tokio::test]
async fn memory_calls_log_no_arguments_and_no_fact_content() {
    let vaults = Vaults::open();
    let db = database().await;
    let scope = physical_scope("device-a");
    vaults
        .remember(&scope, "tiêu đề riêng tư", "thân fact riêng tư")
        .await;
    let memory = admitted(&vaults, &db, "device-a", "physical", StoredMode::Read).await;
    let mut actor = actor_with(memory);

    let log = CapturedLog::default();
    let writer = log.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    let guard = tracing::subscriber::set_default(subscriber);
    let payload = recall(&mut actor, serde_json::json!({"query": "riêng tư"})).await;
    drop(guard);

    assert_eq!(payload["items"][0]["title"], "tiêu đề riêng tư");
    let logged = String::from_utf8(log.0.lock().unwrap().clone()).unwrap();
    assert!(logged.contains("voice_memory_tool_called"), "{logged}");
    assert!(logged.contains("voice_memory_tool_completed"), "{logged}");
    assert!(logged.contains("code=\"ok\""), "{logged}");
    assert!(logged.contains(MEMORY_RECALL_TOOL_NAME), "{logged}");
    assert!(!logged.contains("riêng tư"), "{logged}");
    assert!(!logged.contains("arguments"), "{logged}");
}

/// Prints what the ticket asks to be reported, and asserts nothing: a timing limit would be flaky.
/// `scripts/agent-test.sh run -- cargo test -p voice-agent-server --lib
/// memory_begin_speech_delivery_benchmark -- --ignored --nocapture`
#[tokio::test]
#[ignore = "benchmark; run explicitly and read the printed medians"]
async fn memory_begin_speech_delivery_benchmark() {
    const ITERATIONS: usize = 400;
    let vaults = Vaults::open();
    let db = database().await;
    let scope = physical_scope("device-a");
    // A vault that fills the default 4,000-byte summary: the most Memory can add to a prompt.
    for index in 0..140 {
        vaults
            .remember(
                &scope,
                &format!("Tiêu đề số {index}"),
                &format!("Nội dung của fact số {index}"),
            )
            .await;
    }
    let memory = admitted(&vaults, &db, "device-a", "physical", StoredMode::Read)
        .await
        .unwrap();
    let rows = memory.handle.current().rows.len();
    let bytes: usize = memory.handle.current().rows.iter().map(String::len).sum();

    let measure = |memory: Option<SessionMemory>| {
        let mut samples = Vec::with_capacity(ITERATIONS);
        let mut actor = actor_with(memory);
        for _ in 0..ITERATIONS {
            actor.turn = None;
            actor.begin_active_turn().unwrap();
            let text = "xin chào".to_owned();
            actor.commit_user_text(text.clone()).unwrap();
            let started = std::time::Instant::now();
            actor.begin_speech_delivery(text);
            samples.push(started.elapsed());
            actor.llm_operation = None;
        }
        samples.sort();
        (samples[ITERATIONS / 2], samples[ITERATIONS * 99 / 100])
    };
    // Twice each, so a warm-up or cache effect shows up as a disagreement between the pairs.
    let (first_off_p50, first_off_p99) = measure(None);
    let (first_on_p50, first_on_p99) = measure(Some(memory.clone()));
    let (second_off_p50, second_off_p99) = measure(None);
    let (second_on_p50, second_on_p99) = measure(Some(memory));
    println!(
        "memory_bench rows={rows} row_bytes={bytes} iterations={ITERATIONS}\n\
         memory_bench without_memory p50={first_off_p50:?}/{second_off_p50:?} p99={first_off_p99:?}/{second_off_p99:?}\n\
         memory_bench with_memory    p50={first_on_p50:?}/{second_on_p50:?} p99={first_on_p99:?}/{second_on_p99:?}"
    );
}
