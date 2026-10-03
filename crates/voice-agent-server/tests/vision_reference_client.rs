use axum::Router;
use std::{sync::Arc, time::Duration};
use voice_agent_server::{
    app::{AppState, router_with_state},
    config::AppConfig,
    providers::{ProviderSet, VisionError, VisionProvider, VisionRequest, VisionResponse},
};

struct FixedVision;
#[async_trait::async_trait]
impl VisionProvider for FixedVision {
    fn adapter(&self) -> &'static str {
        "fixed"
    }
    async fn analyze(&self, _: VisionRequest) -> Result<VisionResponse, VisionError> {
        Ok(VisionResponse {
            text: "client-visible result".into(),
        })
    }
}

#[tokio::test]
async fn reference_client_posts_image_to_real_vision_route() {
    let config = AppConfig::parse_and_resolve(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../config.example.toml"
    ))
    .unwrap();
    let app: Router = router_with_state(
        AppState::from_provider_set(config, Arc::new(ProviderSet::unavailable()))
            .with_vision_runtime_for_test(
                "vision-test",
                Arc::new(FixedVision),
                1,
                Duration::from_secs(1),
            ),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let image = std::env::temp_dir().join(format!("vision-client-{}.jpg", std::process::id()));
    tokio::fs::write(&image, [0xff, 0xd8, 0xff, 0xd9])
        .await
        .unwrap();
    let report =
        voice_reference_client::run_vision_request(voice_reference_client::VisionRequestOptions {
            vision_url: format!("http://{address}/mcp/vision/explain"),
            token: String::new(),
            device_id: "device".into(),
            client_id: "client".into(),
            question: "describe".into(),
            image_path: image.clone(),
            timeout: Duration::from_secs(2),
        })
        .await
        .unwrap();
    assert_eq!(report.http_status, 200);
    assert_eq!(report.image_bytes, 4);
    assert_eq!(report.response_text, "client-visible result");
    let _ = tokio::fs::remove_file(image).await;
    server.abort();
}
