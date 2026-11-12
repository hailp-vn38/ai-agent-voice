pub use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

pub use reqwest::{Client, RequestBuilder, Response, StatusCode};
pub use serde_json::{Value, json};
pub use crate::support::admin_auth::admin_web_token;
pub use vela_server::{app::bootstrap_with_providers, config::AppConfig, providers::ProviderSet};

pub struct Server {
    pub base: String,
    pub root: PathBuf,
    pub client: Client,
    pub token: String,
    pub task: tokio::task::JoinHandle<()>,
}

pub fn temp_root() -> PathBuf {
    let root = std::env::temp_dir().join(format!("vela-memory-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    root
}

pub async fn start_at(root: &Path, extra: &str) -> Server {
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
admin_token = "memory-unused-admin-token"
jwt_secret = "memory-harness-signing-secret-0123456789"
[orchestrator]
store_dir = "{root}/orch-store"
tasks_store_dir = "{root}/tasks-store"
reports_dir = "{root}/reports"
work_dir = "{root}/work"
prompts_dir = "{root}/prompts"
skills_dir = "{root}/skills"
{extra}
"#,
            db = root.join("vela.db").display(),
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
    let client = Client::new();
    let token = admin_web_token(&client, &format!("http://{address}")).await;
    Server {
        base: format!("http://{address}/api/admin"),
        root: root.to_path_buf(),
        client,
        token,
        task,
    }
}

pub fn memory_on(root: &Path) -> String {
    format!(
        "[memory]\nenabled = true\nvault_dir = \"{}\"\n",
        root.join("vault").display()
    )
}

pub async fn start() -> Server {
    let root = temp_root();
    let extra = memory_on(&root);
    start_at(&root, &extra).await
}

impl Server {
    pub fn get(&self, path: &str) -> RequestBuilder {
        self.client
            .get(format!("{}{path}", self.base))
            .bearer_auth(&self.token)
    }

    pub fn post(&self, path: &str, body: Value) -> RequestBuilder {
        self.client
            .post(format!("{}{path}", self.base))
            .bearer_auth(&self.token)
            .json(&body)
    }

    pub fn put(&self, path: &str, body: Value) -> RequestBuilder {
        self.client
            .put(format!("{}{path}", self.base))
            .bearer_auth(&self.token)
            .json(&body)
    }

    pub async fn agent(&self, key: &str) {
        ok(self.post("/agents", json!({"key": key, "name": key}))).await;
    }

    pub async fn device(&self, id: &str, agent: &str) -> Value {
        ok(self.post("/devices", json!({"device_id": id, "agent_key": agent}))).await
    }

    /// Creates a fact and returns its id.
    pub async fn remember(&self, device: &str, title: &str, body: &str) -> String {
        let receipt = ok(self.post(
            &format!("/memory/facts?device={device}"),
            json!({"title": title, "hook": "a hook", "body": body, "kind": "user"}),
        ))
        .await;
        receipt["fact_id"].as_str().unwrap().to_string()
    }

    pub async fn facts(&self, device: &str) -> Vec<Value> {
        let listed = ok(self.get(&format!("/memory/facts?device={device}"))).await;
        listed["items"].as_array().unwrap().clone()
    }

    pub fn device_dirs(&self) -> Vec<String> {
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

pub async fn ok(request: RequestBuilder) -> Value {
    let response = request.send().await.unwrap();
    let status = response.status();
    let text = response.text().await.unwrap();
    assert!(status.is_success(), "{status}: {text}");
    serde_json::from_str(&text).unwrap()
}

pub async fn expect_error(response: Response, status: StatusCode, code: &str) {
    assert_eq!(response.status(), status);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["code"], code);
}

pub const FACT_A: &str = "fact_0123456789abcdef0123456789abcdef";

pub fn device_scoped_routes() -> Vec<(reqwest::Method, String, Value)> {
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

/// Waits for what a background task does, without tying the test to how long it takes.
pub async fn eventually<F, Fut>(mut check: F)
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

pub async fn delete_device(server: &Server, device: &str, revision: i64) -> Response {
    server
        .client
        .delete(format!("{}/devices/{device}", server.base))
        .bearer_auth(&server.token)
        .header("if-match", format!("\"{revision}\""))
        .send()
        .await
        .unwrap()
}
