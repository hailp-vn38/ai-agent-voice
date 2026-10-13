use reqwest::{Client, StatusCode};
use serde_json::{Value, json};
use std::sync::Arc;
use voice_agent_server::{
    app::bootstrap_with_providers, config::AppConfig, providers::ProviderSet,
};

#[tokio::test]
async fn background_http_foundation_is_authenticated_and_fails_closed_without_executor() {
    let config: AppConfig = toml::from_str(&format!(
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
url = "sqlite://{}"
[api]
enabled = true
admin_token = "background-test-token"
"#,
        std::env::temp_dir()
            .join(format!("background-http-{}.db", uuid::Uuid::new_v4()))
            .display()
    ))
    .unwrap();
    let router = bootstrap_with_providers(config, Arc::new(ProviderSet::unavailable()))
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/api/admin", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = Client::new();
    assert_eq!(
        client
            .get(format!("{base}/background/settings"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let response = client
        .get(format!("{base}/background/settings"))
        .bearer_auth("background-test-token")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let settings: Value = response.json().await.unwrap();
    assert_eq!(settings["enabled"], false);
    assert_eq!(settings["revision"], 1);
    let response = client
        .post(format!("{base}/jobs"))
        .bearer_auth("background-test-token")
        .json(&json!({"goal":"Collect public AI posts"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        response.json::<Value>().await.unwrap()["error"]["code"],
        "runtime_unavailable"
    );
    server.abort();
}
