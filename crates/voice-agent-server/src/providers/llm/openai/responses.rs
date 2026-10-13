//! One-shot Responses transport for Background Jobs only.
//! Voice streaming continues through the existing Chat Completions implementation.
use super::super::{ChatMessage, ConfiguredOpenAiLlm, LlmError, LlmRequest};
use serde_json::{Value, json};
use std::time::Duration;

const MAX_RESPONSE_BYTES: usize = 256 * 1024;

fn request_body(
    model: &str,
    request: &LlmRequest,
    schema: Option<Value>,
) -> Result<Value, LlmError> {
    if !request.tools.is_empty() {
        return Err(LlmError::Rejected);
    }
    let mut input = Vec::with_capacity(request.messages.len());
    for message in &request.messages {
        let (role, content) = match message {
            ChatMessage::System { content } => ("system", content),
            ChatMessage::User { content } => ("user", content),
            ChatMessage::AssistantText { content } => ("assistant", content),
            ChatMessage::AssistantToolCalls { .. } | ChatMessage::ToolResult { .. } => {
                return Err(LlmError::Rejected);
            }
        };
        input.push(json!({"role":role,"content":content}));
    }
    if input.is_empty() {
        return Err(LlmError::Rejected);
    }
    let format = match schema {
        Some(schema) => {
            json!({"type":"json_schema","name":"agent_job_output","strict":true,"schema":schema})
        }
        None => json!({"type":"json_object"}),
    };
    Ok(json!({
        "model":model,"input":input,"text":{"format":format},
        "store":false,"background":false,"stream":false
    }))
}

fn output_text(body: &Value) -> Result<String, LlmError> {
    if body.get("status").and_then(Value::as_str) != Some("completed") {
        return Err(LlmError::Rejected);
    }
    let messages = body
        .get("output")
        .and_then(Value::as_array)
        .ok_or(LlmError::Rejected)?;
    let mut answer = String::new();
    let mut found = false;
    for item in messages {
        match item.get("type").and_then(Value::as_str) {
            Some("reasoning") => continue,
            Some("message") if item.get("role").and_then(Value::as_str) == Some("assistant") => {
                let parts = item
                    .get("content")
                    .and_then(Value::as_array)
                    .ok_or(LlmError::Rejected)?;
                for part in parts {
                    match part.get("type").and_then(Value::as_str) {
                        Some("output_text") => {
                            let delta = part
                                .get("text")
                                .and_then(Value::as_str)
                                .ok_or(LlmError::Rejected)?;
                            if answer.len().saturating_add(delta.len()) > 65536 {
                                return Err(LlmError::Rejected);
                            }
                            answer.push_str(delta);
                            found = true;
                        }
                        Some("refusal") => return Err(LlmError::Rejected),
                        _ => return Err(LlmError::Rejected),
                    }
                }
            }
            _ => return Err(LlmError::Rejected), // No tool calls or unexpected actions.
        }
    }
    if !found || answer.is_empty() {
        return Err(LlmError::Rejected);
    }
    Ok(answer)
}

pub(crate) async fn complete(
    provider: &ConfiguredOpenAiLlm,
    request: LlmRequest,
    schema: Option<Value>,
) -> Result<String, LlmError> {
    complete_with(
        &provider.base_url,
        &provider.model,
        &provider.api_key,
        provider.timeout_seconds,
        request,
        schema,
    )
    .await
}

/// Shared Responses transport for explicitly opted-in providers.
/// Never injects vendor SDK fingerprints or tool declarations.
pub(crate) async fn complete_with(
    base_url: &str,
    model: &str,
    api_key: &str,
    timeout_seconds: u64,
    request: LlmRequest,
    schema: Option<Value>,
) -> Result<String, LlmError> {
    let base = base_url.trim_end_matches('/');
    let url = format!("{base}/responses");
    let parsed = url::Url::parse(&url).map_err(|_| LlmError::Failed)?;
    if !matches!(parsed.scheme(), "https" | "http")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        return Err(LlmError::Failed);
    }
    let body = request_body(model, &request, schema)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_seconds))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| LlmError::Failed)?;
    let mut send = client.post(parsed).json(&body);
    if !api_key.is_empty() {
        send = send.bearer_auth(api_key);
    }
    let mut response = send.send().await.map_err(|_| LlmError::Failed)?;
    if !response.status().is_success() {
        tracing::warn!(event="background_responses_http_failure",status=%response.status(),"Responses request failed");
        return Err(LlmError::Rejected);
    }
    if response
        .content_length()
        .is_some_and(|size| size > MAX_RESPONSE_BYTES as u64)
    {
        return Err(LlmError::Failed);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| LlmError::Failed)? {
        if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(LlmError::Failed);
        }
        bytes.extend_from_slice(&chunk);
    }
    let document: Value = serde_json::from_slice(&bytes).map_err(|_| LlmError::Rejected)?;
    let text = output_text(&document)?;
    if let Some(usage) = document.get("usage") {
        tracing::info!(
            event = "background_responses_usage",
            input_tokens = usage
                .get("input_tokens")
                .and_then(|value| value.as_u64())
                .unwrap_or(0),
            output_tokens = usage
                .get("output_tokens")
                .and_then(|value| value.as_u64())
                .unwrap_or(0),
            reasoning_tokens = usage
                .pointer("/output_tokens_details/reasoning_tokens")
                .and_then(|value| value.as_u64())
                .unwrap_or(0),
            cached_input_tokens = usage
                .pointer("/input_tokens_details/cached_tokens")
                .and_then(|value| value.as_u64())
                .unwrap_or(0),
        );
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::llm::ToolDefinition;

    fn provider() -> ConfiguredOpenAiLlm {
        ConfiguredOpenAiLlm::build_with_responses(
            "",
            "https://api.openai.com/v1",
            "gpt-test",
            10,
            true,
        )
        .unwrap()
    }
    #[tokio::test]
    async fn enabled_provider_calls_responses_not_chat_completions() {
        use crate::providers::llm::LlmProvider;
        use axum::{Json, Router, routing::post};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/v1", listener.local_addr().unwrap());
        let app = Router::new().route(
            "/v1/responses",
            post(|Json(body): Json<Value>| async move {
                assert_eq!(body["model"], "gpt-test");
                assert_eq!(body["text"]["format"]["type"], "json_object");
                assert_eq!(body["store"], false);
                Json(
                    json!({"status":"completed","output":[{"type":"message","role":"assistant",
                "content":[{"type":"output_text","text":r#"{"steps":[]}"#}]}]}),
                )
            }),
        );
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let p =
            ConfiguredOpenAiLlm::build_with_responses("test", &base, "gpt-test", 5, true).unwrap();
        let answer = p
            .background_complete(LlmRequest::text_turn("JSON plan".into()), None)
            .await
            .unwrap();
        assert_eq!(answer, r#"{"steps":[]}"#);
        task.abort();
    }
    #[tokio::test]
    async fn shared_responses_transport_keeps_opencode_free_model_and_auth() {
        use axum::{Json, Router, http::HeaderMap, routing::post};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/zen/v1", listener.local_addr().unwrap());
        let app = Router::new().route(
            "/zen/v1/responses",
            post(|headers: HeaderMap, Json(body): Json<Value>| async move {
                assert_eq!(headers.get("authorization").unwrap(), "Bearer public");
                assert_eq!(body["model"], "muse-spark-1.3-contributor-free");
                assert_eq!(body["text"]["format"]["type"], "json_schema");
                assert_eq!(body["text"]["format"]["strict"], true);
                assert_eq!(body["store"], false);
                assert_eq!(body["background"], false);
                assert!(body.get("tools").is_none());
                Json(json!({
                    "status":"completed",
                    "output":[{"type":"message","role":"assistant",
                        "content":[{"type":"output_text","text":r#"{"summary":"ok"}"#}]}]
                }))
            }),
        );
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let result = complete_with(
            &base,
            "muse-spark-1.3-contributor-free",
            "public",
            5,
            LlmRequest::text_turn("Summarize in JSON".into()),
            Some(json!({
                "type":"object",
                "properties":{"summary":{"type":"string"}},
                "required":["summary"],
                "additionalProperties":false,
            })),
        )
        .await
        .unwrap();
        assert_eq!(result, r#"{"summary":"ok"}"#);
        server.abort();
    }

    #[test]
    fn response_request_is_job_only_stateless_and_tool_free() {
        let request = LlmRequest {
            messages: vec![
                ChatMessage::System {
                    content: "Return JSON only".into(),
                },
                ChatMessage::User {
                    content: "Summarize this".into(),
                },
            ],
            tools: Vec::new(),
        };
        let body = request_body(
            &provider().model,
            &request,
            Some(json!({
                "type":"object","properties":{"summary":{"type":"string"}},
                "required":["summary"],"additionalProperties":false
            })),
        )
        .unwrap();
        assert_eq!(body["store"], false);
        assert_eq!(body["background"], false);
        assert_eq!(body["text"]["format"]["strict"], true);
        assert_eq!(body["input"][0]["role"], "system");
        assert_eq!(body["input"][1]["role"], "user");
        assert!(body.get("tools").is_none());
    }
    #[test]
    fn plan_request_uses_json_mode_and_rejects_tools() {
        let mut request = LlmRequest::text_turn("Plan JSON".into());
        assert_eq!(
            request_body(&provider().model, &request, None).unwrap()["text"]["format"]["type"],
            "json_object"
        );
        request.tools.push(ToolDefinition {
            name: "danger".into(),
            description: "No".into(),
            parameters: json!({}),
        });
        assert!(request_body(&provider().model, &request, None).is_err());
    }
    #[test]
    fn accepts_completed_text_and_rejects_refusal_and_incomplete() {
        let good = json!({"status":"completed","output":[
            {"type":"reasoning"},
            {"type":"message","role":"assistant","content":[{"type":"output_text","text":"{\"summary\":\"done\"}"}]}
        ]});
        assert_eq!(output_text(&good).unwrap(), r#"{"summary":"done"}"#);
        assert!(output_text(&json!({"status":"incomplete","output":[]})).is_err());
        assert!(output_text(&json!({"status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"refusal","refusal":"No"}]}]})).is_err());
        assert!(
            output_text(
                &json!({"status":"completed","output":[{"type":"function_call","name":"run"}]})
            )
            .is_err()
        );
    }
}
