//! Admin HTTP seam of Memory Core: routes, error codes, the `device` parameter and Device deletion,
//! on a real server with a real database and vault in a temporary directory.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use reqwest::{Client, RequestBuilder, Response, StatusCode};
use serde_json::{Value, json};
use voice_agent_server::{
    app::bootstrap_with_providers, config::AppConfig, providers::ProviderSet,
};

const TOKEN: &str = "admin-test-token";

struct Server {
    base: String,
    root: PathBuf,
    client: Client,
    task: tokio::task::JoinHandle<()>,
}

fn temp_root() -> PathBuf {
    let root = std::env::temp_dir().join(format!("voice-agent-memory-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    root
}

async fn start_at(root: &Path, extra: &str) -> Server {
    let config_path = root.join(format!("config-{}.toml", uuid::Uuid::new_v4()));
    fs::write(
        &config_path,
        format!(
            r#"
[server]
bind = "127.0.0.1:0"
public_ws_url = "ws://127.0.0.1:0/voice/v1/"
[provider_defaults]
vad = "test"
asr = "test"
llm = "test"
tts = "test"
[database]
url = "sqlite://{db}"
[api]
enabled = true
admin_token = "{TOKEN}"
[orchestrator]
store_dir = "{root}/orch-store"
tasks_store_dir = "{root}/tasks-store"
reports_dir = "{root}/reports"
work_dir = "{root}/work"
prompts_dir = "{root}/prompts"
skills_dir = "{root}/skills"
{extra}
"#,
            db = root.join("voice-agent.db").display(),
            root = root.display(),
        ),
    )
    .unwrap();
    let config = AppConfig::parse_and_resolve(&config_path).unwrap();
    let router = bootstrap_with_providers(config, Arc::new(ProviderSet::unavailable()))
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    Server {
        base: format!("http://{address}/api/admin"),
        root: root.to_path_buf(),
        client: Client::new(),
        task,
    }
}

fn memory_on(root: &Path) -> String {
    format!(
        "[memory]\nenabled = true\nvault_dir = \"{}\"\n",
        root.join("vault").display()
    )
}

async fn start() -> Server {
    let root = temp_root();
    let extra = memory_on(&root);
    start_at(&root, &extra).await
}

impl Server {
    fn get(&self, path: &str) -> RequestBuilder {
        self.client
            .get(format!("{}{path}", self.base))
            .bearer_auth(TOKEN)
    }

    fn post(&self, path: &str, body: Value) -> RequestBuilder {
        self.client
            .post(format!("{}{path}", self.base))
            .bearer_auth(TOKEN)
            .json(&body)
    }

    fn put(&self, path: &str, body: Value) -> RequestBuilder {
        self.client
            .put(format!("{}{path}", self.base))
            .bearer_auth(TOKEN)
            .json(&body)
    }

    async fn agent(&self, key: &str) {
        ok(self.post("/agents", json!({"key": key, "name": key}))).await;
    }

    async fn device(&self, id: &str, agent: &str) -> Value {
        ok(self.post("/devices", json!({"device_id": id, "agent_key": agent}))).await
    }

    /// Creates a fact and returns its id.
    async fn remember(&self, device: &str, title: &str, body: &str) -> String {
        let receipt = ok(self.post(
            &format!("/memory/facts?device={device}"),
            json!({"title": title, "hook": "a hook", "body": body, "kind": "user"}),
        ))
        .await;
        receipt["fact_id"].as_str().unwrap().to_string()
    }

    async fn facts(&self, device: &str) -> Vec<Value> {
        let listed = ok(self.get(&format!("/memory/facts?device={device}"))).await;
        listed["items"].as_array().unwrap().clone()
    }

    fn device_dirs(&self) -> Vec<String> {
        let Ok(entries) = fs::read_dir(self.root.join("vault/devices")) else {
            return Vec::new();
        };
        let mut names: Vec<String> = entries
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }
}

async fn ok(request: RequestBuilder) -> Value {
    let response = request.send().await.unwrap();
    let status = response.status();
    let text = response.text().await.unwrap();
    assert!(status.is_success(), "{status}: {text}");
    serde_json::from_str(&text).unwrap()
}

async fn expect_error(response: Response, status: StatusCode, code: &str) {
    assert_eq!(response.status(), status);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["code"], code);
}

const FACT_A: &str = "fact_0123456789abcdef0123456789abcdef";

fn device_scoped_routes() -> Vec<(reqwest::Method, String, Value)> {
    use reqwest::Method;
    let fact = json!({"title": "t", "hook": "h", "body": "b", "kind": "user"});
    let revision = json!({"expected_revision": 1});
    vec![
        (Method::GET, "/memory/facts".into(), Value::Null),
        (Method::POST, "/memory/facts".into(), fact),
        (Method::GET, format!("/memory/facts/{FACT_A}"), Value::Null),
        (
            Method::PUT,
            format!("/memory/facts/{FACT_A}"),
            json!({"body": "b", "expected_revision": 1}),
        ),
        (
            Method::POST,
            format!("/memory/facts/{FACT_A}/forget"),
            revision.clone(),
        ),
        (
            Method::POST,
            format!("/memory/facts/{FACT_A}/discard"),
            revision,
        ),
        (
            Method::POST,
            format!("/memory/facts/{FACT_A}/pin"),
            json!({"pinned": true}),
        ),
        (Method::GET, "/memory/summary".into(), Value::Null),
        (Method::GET, "/memory/index".into(), Value::Null),
        (Method::POST, "/memory/index/rebuild".into(), Value::Null),
    ]
}

#[tokio::test]
async fn memory_routes_need_a_device_that_exists_and_create_no_vault_otherwise() {
    let server = start().await;
    for (method, path, body) in device_scoped_routes() {
        let url = format!("{}{path}", server.base);
        let send = |url: String| {
            let request = server
                .client
                .request(method.clone(), url)
                .bearer_auth(TOKEN);
            if body.is_null() {
                request.send()
            } else {
                request.json(&body).send()
            }
        };
        let missing = send(url.clone()).await.unwrap();
        assert_eq!(missing.status(), StatusCode::BAD_REQUEST, "{method} {path}");
        let empty = send(format!("{url}?device=")).await.unwrap();
        assert_eq!(empty.status(), StatusCode::BAD_REQUEST, "{method} {path}");
        expect_error(
            send(format!("{url}?device=ghost")).await.unwrap(),
            StatusCode::NOT_FOUND,
            "device_not_found",
        )
        .await;
    }
    assert!(server.device_dirs().is_empty());
    server.task.abort();
}

#[tokio::test]
async fn memory_disabled_answers_503_touches_no_disk_and_leaves_the_server_running() {
    let root = temp_root();
    let server = start_at(&root, "").await;
    server.agent("home").await;
    server.device("kitchen", "home").await;
    expect_error(
        server
            .get("/memory/facts?device=kitchen")
            .send()
            .await
            .unwrap(),
        StatusCode::SERVICE_UNAVAILABLE,
        "memory_disabled",
    )
    .await;
    expect_error(
        server.get("/memory/profile").send().await.unwrap(),
        StatusCode::SERVICE_UNAVAILABLE,
        "memory_disabled",
    )
    .await;
    assert!(!root.join("vault").exists());
    server.task.abort();
}

#[tokio::test]
async fn memory_that_cannot_open_keeps_the_server_up_and_reports_unavailable() {
    let root = temp_root();
    // A file where the vault directory should be.
    fs::write(root.join("vault"), b"not a directory").unwrap();
    let extra = memory_on(&root);
    let server = start_at(&root, &extra).await;
    server.agent("home").await;
    server.device("kitchen", "home").await;
    expect_error(
        server
            .get("/memory/facts?device=kitchen")
            .send()
            .await
            .unwrap(),
        StatusCode::SERVICE_UNAVAILABLE,
        "memory_unavailable",
    )
    .await;
    server.task.abort();
}

#[tokio::test]
async fn memory_fact_lifecycle_reports_real_state_on_a_device_that_is_off() {
    let server = start().await;
    server.agent("home").await;
    let device = server.device("kitchen", "home").await;
    assert_eq!(device["memory_mode"], "off");

    let created = server
        .post(
            "/memory/facts?device=kitchen",
            json!({"title": "Lịch họp", "hook": "họp thứ hai", "body": "Đặt lịch họp thứ hai", "kind": "business"}),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let receipt: Value = created.json().await.unwrap();
    assert_eq!(receipt["action"], "remember");
    assert_eq!(receipt["revision"], 1);
    assert_eq!(receipt["cleanup_pending"], false);
    let id = receipt["fact_id"].as_str().unwrap().to_string();
    assert!(id.starts_with("fact_"));

    // The same body again returns the fact it already has.
    let again = server
        .remember("kitchen", "Lịch khác", "Đặt lịch họp thứ hai")
        .await;
    assert_eq!(again, id);

    let items = server.facts("kitchen").await;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"], id.as_str());
    assert_eq!(items[0]["revision"], 1);
    assert_eq!(items[0]["status"], "confirmed");
    assert_eq!(items[0]["kind"], "business");
    assert_eq!(items[0]["pinned"], false);
    assert_eq!(items[0]["hook"], "họp thứ hai");

    // Search folds Vietnamese, as ASR text arrives without diacritics.
    let found = ok(server.get("/memory/facts?device=kitchen&q=dat%20lich")).await;
    assert_eq!(found["items"][0]["id"], id.as_str());
    let nothing = ok(server.get("/memory/facts?device=kitchen&q=zzzqqq")).await;
    assert_eq!(nothing["items"], json!([]));

    let fact_path = format!("/memory/facts/{id}?device=kitchen");
    let read = ok(server.get(&fact_path)).await;
    assert_eq!(read["body"], "Đặt lịch họp thứ hai");

    // `expected_revision` is mandatory, and an old one changes nothing.
    assert_eq!(
        server
            .put(&fact_path, json!({"body": "x"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    expect_error(
        server
            .put(&fact_path, json!({"body": "x", "expected_revision": 9}))
            .send()
            .await
            .unwrap(),
        StatusCode::CONFLICT,
        "memory_revision_conflict",
    )
    .await;
    assert_eq!(
        ok(server.get(&fact_path)).await["body"],
        "Đặt lịch họp thứ hai"
    );
    let corrected = ok(server.put(
        &fact_path,
        json!({"title": "Lịch mới", "hook": "thứ ba", "body": "Đặt lịch họp thứ ba", "expected_revision": 1}),
    ))
    .await;
    assert_eq!(corrected["action"], "correct");
    assert_eq!(corrected["revision"], 2);
    let read = ok(server.get(&fact_path)).await;
    assert_eq!(read["title"], "Lịch mới");
    assert_eq!(read["hook"], "thứ ba");

    // Pinning keeps the revision.
    let pin_path = format!("/memory/facts/{id}/pin?device=kitchen");
    ok(server.post(&pin_path, json!({"pinned": true}))).await;
    let read = ok(server.get(&fact_path)).await;
    assert_eq!(read["pinned"], true);
    assert_eq!(read["revision"], 2);

    let summary = ok(server.get("/memory/summary?device=kitchen")).await;
    assert!(summary["summary"].as_str().unwrap().contains("Lịch mới"));
    assert_eq!(summary["hidden"], 0);

    let index = ok(server.get("/memory/index?device=kitchen")).await;
    assert_eq!(index["facts"], 1);
    assert_eq!(index["quarantined"], 0);
    assert_eq!(index["memory_mode"], "off");
    assert_eq!(
        ok(server.post("/memory/index/rebuild?device=kitchen", json!({}))).await["facts"],
        1
    );

    // Forget needs the current revision and leaves a tombstone.
    let forget_path = format!("/memory/facts/{id}/forget?device=kitchen");
    assert_eq!(
        server
            .post(&forget_path, json!({}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    expect_error(
        server
            .post(&forget_path, json!({"expected_revision": 1}))
            .send()
            .await
            .unwrap(),
        StatusCode::CONFLICT,
        "memory_revision_conflict",
    )
    .await;
    let forgotten = ok(server.post(&forget_path, json!({"expected_revision": 2}))).await;
    assert_eq!(forgotten["action"], "forget");
    assert!(server.facts("kitchen").await.is_empty());
    expect_error(
        server.get(&fact_path).send().await.unwrap(),
        StatusCode::NOT_FOUND,
        "memory_not_found",
    )
    .await;
    let summary = ok(server.get("/memory/summary?device=kitchen")).await;
    assert_eq!(summary["summary"], "");
    server.task.abort();
}

#[tokio::test]
async fn memory_discard_leaves_no_tombstone_so_the_same_content_saves_again() {
    let server = start().await;
    server.agent("home").await;
    server.device("kitchen", "home").await;
    let id = server
        .remember("kitchen", "Ghi nhầm", "Nội dung nhầm")
        .await;
    let discarded = ok(server.post(
        &format!("/memory/facts/{id}/discard?device=kitchen"),
        json!({"expected_revision": 1}),
    ))
    .await;
    assert_eq!(discarded["action"], "discard");
    assert!(server.facts("kitchen").await.is_empty());
    for dir in server.device_dirs() {
        let path = server
            .root
            .join("vault/devices")
            .join(dir)
            .join("tombstones.json");
        assert!(!path.exists(), "Discard wrote a Forget Tombstone");
    }
    let again = server
        .remember("kitchen", "Ghi nhầm", "Nội dung nhầm")
        .await;
    assert_ne!(again, id);
    server.task.abort();
}

#[tokio::test]
async fn memory_rejects_invalid_fact_input_and_maps_core_errors() {
    let server = start().await;
    server.agent("home").await;
    server.device("kitchen", "home").await;
    let create = "/memory/facts?device=kitchen";
    for body in [
        json!({"title": " ", "hook": "h", "body": "b", "kind": "user"}),
        json!({"title": "t", "hook": "h", "body": " ", "kind": "user"}),
        json!({"title": "t", "hook": "h", "body": "b", "kind": "secret"}),
        json!({"title": "t", "hook": "h", "body": "b"}),
    ] {
        assert_eq!(
            server.post(create, body).send().await.unwrap().status(),
            StatusCode::BAD_REQUEST
        );
    }
    expect_error(
        server
            .post(
                create,
                json!({"title": "t", "hook": "h", "kind": "user",
                       "body": "postgres://admin:hunter2@db.example.com/prod"}),
            )
            .send()
            .await
            .unwrap(),
        StatusCode::BAD_REQUEST,
        "memory_rejected_sensitive",
    )
    .await;
    expect_error(
        server
            .post(
                create,
                json!({"title": "t", "hook": "h", "kind": "user", "body": "x".repeat(2_001)}),
            )
            .send()
            .await
            .unwrap(),
        StatusCode::BAD_REQUEST,
        "memory_too_large",
    )
    .await;
    server.task.abort();
}

#[tokio::test]
async fn memory_is_kept_apart_per_device_and_follows_a_device_to_a_new_agent() {
    let server = start().await;
    server.agent("home").await;
    server.agent("office").await;
    let kitchen = server.device("kitchen", "home").await;
    server.device("bedroom", "home").await;
    let id = server
        .remember("kitchen", "Bí mật bếp", "Chỉ bếp biết")
        .await;

    assert!(server.facts("bedroom").await.is_empty());
    let searched = ok(server.get("/memory/facts?device=bedroom&q=bep")).await;
    assert_eq!(searched["items"], json!([]));
    expect_error(
        server
            .get(&format!("/memory/facts/{id}?device=bedroom"))
            .send()
            .await
            .unwrap(),
        StatusCode::NOT_FOUND,
        "memory_not_found",
    )
    .await;

    let revision = kitchen["revision"].as_i64().unwrap();
    let moved = ok(server
        .client
        .patch(format!("{}/devices/kitchen", server.base))
        .bearer_auth(TOKEN)
        .header("if-match", format!("\"{revision}\""))
        .json(&json!({"agent_key": "office"})))
    .await;
    assert_eq!(moved["agent_key"], "office");
    let items = server.facts("kitchen").await;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"], id.as_str());
    server.task.abort();
}

#[tokio::test]
async fn memory_of_the_virtual_device_lives_under_its_own_directory() {
    let server = start().await;
    server.agent("home").await;
    let settings = ok(server.put(
        "/system/assistant-device",
        json!({"enabled": true, "agent_key": "home", "template_key": null, "memory_mode": "read"}),
    ))
    .await;
    assert_eq!(settings["memory_mode"], "read");
    let device = settings["device_id"].as_str().unwrap();
    server
        .remember(device, "Trợ lý", "Ghi chú của trợ lý")
        .await;
    assert_eq!(server.device_dirs(), vec!["web-assistant".to_string()]);
    assert_eq!(server.facts(device).await.len(), 1);
    server.task.abort();
}

#[tokio::test]
async fn memory_profile_needs_no_device_and_checks_its_revision() {
    let server = start().await;
    let initial = ok(server.get("/memory/profile")).await;
    assert_eq!(initial["body"], "");
    let revision = initial["revision"].as_u64().unwrap();

    let saved = ok(server.post(
        "/memory/profile",
        json!({"body": "Công ty ở Đà Nẵng", "expected_revision": revision}),
    ))
    .await;
    assert_eq!(saved["action"], "set_profile");
    let read = ok(server.get("/memory/profile")).await;
    assert_eq!(read["body"], "Công ty ở Đà Nẵng");
    assert_eq!(read["revision"], revision + 1);

    expect_error(
        server
            .post(
                "/memory/profile",
                json!({"body": "cũ", "expected_revision": revision}),
            )
            .send()
            .await
            .unwrap(),
        StatusCode::CONFLICT,
        "memory_revision_conflict",
    )
    .await;

    // A Device's summary carries the shared profile.
    server.agent("home").await;
    server.device("kitchen", "home").await;
    let summary = ok(server.get("/memory/summary?device=kitchen")).await;
    assert!(summary["summary"].as_str().unwrap().contains("Đà Nẵng"));
    server.task.abort();
}

#[tokio::test]
async fn memory_routes_never_reach_a_wiki_page_and_wiki_search_never_sees_a_fact() {
    let root = temp_root();
    let extra = format!(
        "{}[wiki]\nenabled = true\nvault_dir = \"{}\"\n",
        memory_on(&root),
        root.join("wiki").display()
    );
    let server = start_at(&root, &extra).await;
    server.agent("home").await;
    server.device("kitchen", "home").await;

    let page = ok(server.post(
        "/knowledge/pages",
        json!({"title": "Wiki tri thức", "body": "Trang wiki còn nguyên", "source_ref": "test"}),
    ))
    .await;
    let page_id = page["id"].as_str().unwrap().to_string();
    let encoded = page_id.replace('/', "%2F");

    for suffix in ["forget", "discard"] {
        expect_error(
            server
                .post(
                    &format!("/memory/facts/{encoded}/{suffix}?device=kitchen"),
                    json!({"expected_revision": 1}),
                )
                .send()
                .await
                .unwrap(),
            StatusCode::NOT_FOUND,
            "memory_not_found",
        )
        .await;
    }
    expect_error(
        server
            .get(&format!("/memory/facts/{encoded}?device=kitchen"))
            .send()
            .await
            .unwrap(),
        StatusCode::NOT_FOUND,
        "memory_not_found",
    )
    .await;
    let still_there = ok(server.get(&format!("/knowledge/pages/{encoded}"))).await;
    assert!(still_there.to_string().contains("Trang wiki"));

    server
        .remember("kitchen", "Mật danh", "zephyrquokka là mật danh")
        .await;
    let search = ok(server.get("/knowledge/pages?q=zephyrquokka")).await;
    assert_eq!(search["items"], json!([]));
    // The Memory screen lists only Memory, whatever the Wiki holds.
    assert_eq!(server.facts("kitchen").await.len(), 1);
    server.task.abort();
}

/// Waits for what a background task does, without tying the test to how long it takes.
async fn eventually<F, Fut>(mut check: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for _ in 0..200 {
        if check().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("condition was not reached in time");
}

async fn delete_device(server: &Server, device: &str, revision: i64) -> Response {
    server
        .client
        .delete(format!("{}/devices/{device}", server.base))
        .bearer_auth(TOKEN)
        .header("if-match", format!("\"{revision}\""))
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn memory_vault_is_purged_only_after_the_device_delete_succeeds() {
    let server = start().await;
    server.agent("home").await;
    let doomed = server.device("doomed", "home").await;
    let keeper = server.device("keeper", "home").await;
    server
        .remember("doomed", "Của doomed", "Nội dung doomed")
        .await;
    server
        .remember("keeper", "Của keeper", "Nội dung keeper")
        .await;
    let before = server.device_dirs();
    assert_eq!(before.len(), 2);

    // A stale revision refuses the delete, so the vault stays.
    let stale = delete_device(&server, "doomed", 99).await;
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    assert_eq!(server.device_dirs(), before);
    assert_eq!(server.facts("doomed").await.len(), 1);

    // So does `device_in_use`, which is what history on the Device means.
    let pool = sqlx::SqlitePool::connect(&format!(
        "sqlite://{}",
        server.root.join("voice-agent.db").display()
    ))
    .await
    .unwrap();
    let keeper_row = keeper["id"].as_i64().unwrap();
    sqlx::query(
        "INSERT INTO history_messages \
         (session_id, device_id, agent_id, template_id, sequence, turn_id, role, text, created_at) \
         SELECT 's', id, agent_id, NULL, 1, '1', 'user', 'x', 1 FROM devices WHERE id = ?",
    )
    .bind(keeper_row)
    .execute(&pool)
    .await
    .unwrap();
    let in_use = delete_device(&server, "keeper", keeper["revision"].as_i64().unwrap()).await;
    expect_error(in_use, StatusCode::CONFLICT, "device_in_use").await;
    assert_eq!(server.device_dirs(), before);
    assert_eq!(server.facts("keeper").await.len(), 1);

    // A successful delete takes the vault with it, and only that vault.
    let deleted = delete_device(&server, "doomed", doomed["revision"].as_i64().unwrap()).await;
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    assert_eq!(server.device_dirs().len(), 1);
    assert_eq!(server.facts("keeper").await.len(), 1);
    expect_error(
        server
            .get("/memory/facts?device=doomed")
            .send()
            .await
            .unwrap(),
        StatusCode::NOT_FOUND,
        "device_not_found",
    )
    .await;

    // Enrolling the same id again starts empty.
    server.device("doomed", "home").await;
    assert!(server.facts("doomed").await.is_empty());
    server.task.abort();
}

#[tokio::test]
async fn memory_managed_device_refuses_delete_and_keeps_its_vault() {
    let server = start().await;
    server.agent("home").await;
    let settings = ok(server.put(
        "/system/assistant-device",
        json!({"enabled": true, "agent_key": "home", "template_key": null}),
    ))
    .await;
    let device = settings["device_id"].as_str().unwrap();
    server
        .remember(device, "Trợ lý", "Ghi chú của trợ lý")
        .await;
    let refused = delete_device(&server, device, settings["revision"].as_i64().unwrap()).await;
    expect_error(
        refused,
        StatusCode::CONFLICT,
        "managed_device_use_system_settings",
    )
    .await;
    assert_eq!(server.facts(device).await.len(), 1);
    server.task.abort();
}

#[tokio::test]
async fn memory_startup_reconciles_in_the_background_and_only_reports_orphans() {
    let root = temp_root();
    let extra = memory_on(&root);
    let server = start_at(&root, &extra).await;
    server.agent("home").await;
    server.device("kitchen", "home").await;
    let id = server.remember("kitchen", "Lịch", "Nội dung gốc").await;
    let dir = server.device_dirs().remove(0);
    server.task.abort();

    // While the server is down: an owner edit, and a directory no Device owns.
    let facts_dir = root.join("vault/devices").join(&dir).join("facts");
    let fact_path = fs::read_dir(&facts_dir)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let text = fs::read_to_string(&fact_path).unwrap();
    fs::write(&fact_path, text.replace("Nội dung gốc", "Nội dung đã sửa")).unwrap();
    let orphan = root.join("vault/devices/0000000000000000");
    fs::create_dir_all(orphan.join("facts")).unwrap();
    fs::write(orphan.join("facts/giu-lai.md"), "giữ lại").unwrap();

    let server = start_at(&root, &extra).await;
    // Convergence is visible without anyone asking for it.
    let path = format!("/memory/facts/{id}?device=kitchen");
    eventually(|| async { ok(server.get(&path)).await["revision"] == 2 }).await;
    let fact = ok(server.get(&path)).await;
    assert_eq!(fact["origin"], "owner-edited");
    assert_eq!(fact["body"], "Nội dung đã sửa");
    assert_eq!(
        fs::read_to_string(orphan.join("facts/giu-lai.md")).unwrap(),
        "giữ lại"
    );
    server.task.abort();
}
