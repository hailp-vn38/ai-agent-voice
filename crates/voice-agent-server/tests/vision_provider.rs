use axum::{Json, Router, routing::post};
use std::sync::Arc;
use voice_agent_server::{
    config::{OpenAiVisionConfig, SecretString},
    providers::{OpenAiVisionProvider, VisionProvider, VisionRequest},
};

async fn upstream(Json(payload): Json<serde_json::Value>) -> Json<serde_json::Value> {
    assert_eq!(payload["model"], "vision-model");
    assert_eq!(payload["stream"], false);
    assert_eq!(payload["messages"][0]["content"][0]["text"], "describe");
    assert_eq!(
        payload["messages"][0]["content"][1]["image_url"]["url"],
        "data:image/png;base64,iVBORw=="
    );
    Json(serde_json::json!({"choices":[{"message":{"content":"  vision result  "}}]}))
}
#[tokio::test]
async fn openai_vision_sends_multimodal_payload_over_http() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new().route("/v1/chat/completions", post(upstream)),
        )
        .await
        .unwrap();
    });
    let provider = OpenAiVisionProvider::new(OpenAiVisionConfig {
        base_url: format!("http://{address}/v1").parse().unwrap(),
        api_key: SecretString::new("upstream-key"),
        model: "vision-model".into(),
        timeout_ms: 1000,
        max_tokens: 12,
        temperature: 0.2,
        top_p: 1.0,
    })
    .unwrap();
    let response = provider
        .analyze(VisionRequest {
            question: Arc::from("describe"),
            image: Arc::from([137, 80, 78, 71].as_slice()),
            mime_type: Arc::from("image/png"),
        })
        .await
        .unwrap();
    assert_eq!(response.text, "vision result");
    task.abort();
}
