use std::{fs, sync::Arc};

use reqwest::{Client, Method, StatusCode};
use voice_agent_server::{
    app::bootstrap_with_providers, config::AppConfig, providers::ProviderSet,
};

async fn server() -> (String, tokio::task::JoinHandle<()>) {
    let path = std::env::temp_dir().join(format!("voice-cors-{}.toml", uuid::Uuid::new_v4()));
    fs::write(
        &path,
        r#"
[server]
bind = "127.0.0.1:0"
public_ws_url = "ws://192.168.1.158:8000/voice/v1/"
[provider_defaults]
vad = "test"
asr = "test"
llm = "test"
tts = "test"
[database]
url = "sqlite::memory:"
[api]
enabled = true
admin_token = "admin-test-token"
"#,
    )
    .unwrap();
    let config = AppConfig::parse_and_resolve(&path).unwrap();
    fs::remove_file(path).unwrap();
    let router = bootstrap_with_providers(config, Arc::new(ProviderSet::unavailable()))
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (base, task)
}

#[tokio::test]
async fn lan_browser_preflight_and_authenticated_requests() {
    let (base, task) = server().await;
    let client = Client::new();
    let origin = "http://192.168.1.158:5173";
    for method in ["GET", "POST", "PUT", "PATCH", "DELETE"] {
        let response = client
            .request(Method::OPTIONS, format!("{base}/api/admin/agents"))
            .header("Origin", origin)
            .header("Access-Control-Request-Method", method)
            .header(
                "Access-Control-Request-Headers",
                "authorization,content-type,if-match",
            )
            .send()
            .await
            .unwrap();
        assert!(
            response.status().is_success(),
            "preflight: {}",
            response.status()
        );
        assert_eq!(response.headers()["access-control-allow-origin"], origin);
        assert!(
            response.headers()["access-control-allow-methods"]
                .to_str()
                .unwrap()
                .contains(method)
        );
        let headers = response.headers()["access-control-allow-headers"]
            .to_str()
            .unwrap()
            .to_ascii_lowercase();
        for header in ["authorization", "content-type", "if-match"] {
            assert!(headers.contains(header));
        }
    }
    for token in [None, Some("admin-test-token")] {
        let mut request = client
            .get(format!("{base}/api/admin/agents"))
            .header("Origin", origin);
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        let response = request.send().await.unwrap();
        assert_eq!(
            response.status(),
            if token.is_some() {
                StatusCode::OK
            } else {
                StatusCode::UNAUTHORIZED
            }
        );
        assert_eq!(response.headers()["access-control-allow-origin"], origin);
    }
    for route in ["/health", "/ready", "/voice/ota/"] {
        let response = client
            .get(format!("{base}{route}"))
            .header("Origin", origin)
            .send()
            .await
            .unwrap();
        assert_eq!(
            response
                .headers()
                .get("access-control-allow-origin")
                .and_then(|value| value.to_str().ok()),
            Some(origin),
            "{route}"
        );
    }
    let response = client
        .request(Method::OPTIONS, format!("{base}/voice/ota/"))
        .header("Origin", origin)
        .header("Access-Control-Request-Method", "GET")
        .header("Access-Control-Request-Headers", "device-id,client-id")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(response.headers()["access-control-allow-origin"], origin);
    assert!(
        response.headers()["access-control-allow-methods"]
            .to_str()
            .unwrap()
            .contains("GET")
    );
    task.abort();
}

#[tokio::test]
async fn public_and_opaque_origins_are_not_allowed_for_admin() {
    let (base, task) = server().await;
    for origin in ["https://example.com", "http://8.8.8.8:5173", "null"] {
        let response = Client::new()
            .get(format!("{base}/api/admin/agents"))
            .header("Origin", origin)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(
            !response
                .headers()
                .contains_key("access-control-allow-origin")
        );
    }
    task.abort();
}

#[tokio::test]
async fn local_ipv4_and_ipv6_origins_are_allowed() {
    let (base, task) = server().await;
    let client = Client::new();
    for origin in [
        "http://localhost:5173",
        "http://127.0.0.1:5173",
        "http://10.0.0.2:5173",
        "https://172.16.0.2:5173",
        "http://169.254.1.2:5173",
        "http://[::1]:5173",
        "http://[fd00::2]:5173",
        "http://[fe80::2]:5173",
    ] {
        let response = client
            .get(format!("{base}/api/admin/agents"))
            .bearer_auth("admin-test-token")
            .header("Origin", origin)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["access-control-allow-origin"], origin);
        assert!(
            response.headers()["access-control-expose-headers"]
                .to_str()
                .unwrap()
                .contains("x-request-id")
        );
        assert!(
            !response
                .headers()
                .contains_key("access-control-allow-credentials")
        );
    }
    task.abort();
}
