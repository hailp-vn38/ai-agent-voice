use axum::Router;
use std::{sync::Arc, time::Duration};
use voice_agent_server::{
    app::{AppState, router_with_state},
    config::AppConfig,
    providers::{ProviderSet, VisionError, VisionProvider, VisionRequest, VisionResponse},
};

struct EchoVision;
#[async_trait::async_trait]
impl VisionProvider for EchoVision {
    fn adapter(&self) -> &'static str {
        "echo_vision"
    }
    async fn analyze(&self, request: VisionRequest) -> Result<VisionResponse, VisionError> {
        Ok(VisionResponse {
            text: format!("{}:{}", request.mime_type, request.image.len()),
        })
    }
}
async fn spawn(app: Router) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{address}"), task)
}
fn app() -> Router {
    let config = AppConfig::parse_and_resolve(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../config.example.toml"
    ))
    .unwrap();
    router_with_state(
        AppState::from_provider_set(config, Arc::new(ProviderSet::unavailable()))
            .with_vision_runtime_for_test(
                "vision-test",
                Arc::new(EchoVision),
                1,
                Duration::from_secs(1),
            ),
    )
}
#[tokio::test]
async fn vision_api_get_probe_and_multipart_post_work_over_tcp() {
    let (base, task) = spawn(app()).await;
    let probe = reqwest::get(format!("{base}/mcp/vision/explain"))
        .await
        .unwrap();
    assert_eq!(probe.status(), reqwest::StatusCode::OK);
    let image = vec![0xff, 0xd8, 0xff, 0xd9];
    let response = reqwest::Client::new()
        .post(format!("{base}/mcp/vision/explain"))
        .header("Device-Id", "device")
        .header("Client-Id", "client")
        .multipart(
            reqwest::multipart::Form::new()
                .part(
                    "image",
                    reqwest::multipart::Part::bytes(image).file_name("image.jpg"),
                )
                .text("question", "what is this?"),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(
        response.json::<serde_json::Value>().await.unwrap()["response"],
        "image/jpeg:4"
    );
    task.abort();
}

#[tokio::test]
async fn vision_api_options_preflight_works_over_tcp() {
    let (base, task) = spawn(app()).await;
    let response = reqwest::Client::new()
        .request(
            reqwest::Method::OPTIONS,
            format!("{base}/mcp/vision/explain"),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::NO_CONTENT);
    assert_eq!(
        response.headers()["access-control-allow-methods"],
        "GET, POST, OPTIONS"
    );
    task.abort();
}
#[tokio::test]
async fn vision_api_rejects_invalid_public_contract_inputs() {
    let (base, task) = spawn(app()).await;
    let client = reqwest::Client::new();
    let missing_headers = client
        .post(format!("{base}/mcp/vision/explain"))
        .multipart(reqwest::multipart::Form::new().text("question", "q"))
        .send()
        .await
        .unwrap();
    assert_eq!(missing_headers.status(), reqwest::StatusCode::BAD_REQUEST);
    let invalid_image = client
        .post(format!("{base}/mcp/vision/explain"))
        .header("Device-Id", "d")
        .header("Client-Id", "c")
        .multipart(
            reqwest::multipart::Form::new()
                .text("question", "q")
                .part("image", reqwest::multipart::Part::bytes(vec![1, 2, 3])),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(
        invalid_image.status(),
        reqwest::StatusCode::UNSUPPORTED_MEDIA_TYPE
    );
    task.abort();
}
