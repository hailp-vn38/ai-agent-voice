use axum::{Json, Router, routing::post};
use voice_agent_server::{
    config::{LlmInstanceConfig, OpenAiConfig, SecretString},
    providers::{LlmOperation, LlmOutput, compiled_provider_registry},
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
async fn openai_llm_with_vision_tag_sends_multimodal_payload_over_http() {
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
    let provider = compiled_provider_registry()
        .llm_factory("openai")
        .unwrap()
        .build(&LlmInstanceConfig::Openai(OpenAiConfig {
            base_url: format!("http://{address}/v1").parse().unwrap(),
            api_key: SecretString::new("upstream-key"),
            model: "vision-model".into(),
            timeout_ms: 1000,
            max_tokens: Some(256),
            temperature_percent: Some(20),
            tags: Some(vec!["chat".into(), "vision".into()]),
            ..Default::default()
        }))
        .unwrap();
    let response = provider
        .invoke(LlmOperation::Vision {
            prompt: "describe".into(),
            image_bytes: vec![137, 80, 78, 71],
            mime_type: "image/png".into(),
        })
        .await
        .unwrap();
    assert_eq!(response, LlmOutput::Text("vision result".into()));
    task.abort();
}
