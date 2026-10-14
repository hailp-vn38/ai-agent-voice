//! One-shot Responses transport for Background Jobs only.
//! Voice streaming continues through the existing Chat Completions implementation.
use super::super::{LlmError, LlmRequest};
use serde_json::{Value, json};
use std::time::Duration;

const MAX_RESPONSE_BYTES: usize = 256 * 1024;

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

fn turn_body(
    model: &str,
    request: &LlmRequest,
    mode: &crate::providers::llm::BackgroundMode,
) -> Result<Value, LlmError> {
    use crate::providers::llm::{BackgroundMode, ChatMessage};
    let mut input = Vec::new();
    for message in &request.messages {
        match message {
            ChatMessage::System { content } => {
                input.push(json!({"role":"system","content":content}))
            }
            ChatMessage::User { content } => input.push(json!({"role":"user","content":content})),
            ChatMessage::AssistantText { content } => {
                input.push(json!({"role":"assistant","content":content}))
            }
            ChatMessage::AssistantToolCalls { calls } => {
                for call in calls {
                    input.push(
                        json!({"type":"function_call","call_id":call.id,"name":call.name,
                        "arguments":call.arguments.to_string()}),
                    );
                }
            }
            ChatMessage::ToolResult {
                tool_call_id,
                content,
            } => input.push(json!({
                "type":"function_call_output","call_id":tool_call_id,"output":content
            })),
        }
    }
    if input.is_empty() {
        return Err(LlmError::Rejected);
    }
    let mut body =
        json!({"model":model,"input":input,"store":false,"background":false,"stream":false});
    match mode {
        BackgroundMode::Structured { name, schema } => {
            if !request.tools.is_empty() {
                return Err(LlmError::Rejected);
            }
            body["text"] =
                json!({"format":{"type":"json_schema","name":name,"strict":true,"schema":schema}});
        }
        BackgroundMode::Tools | BackgroundMode::ToolsAuto | BackgroundMode::ToolsParallel => {
            if request.tools.is_empty() {
                return Err(LlmError::Rejected);
            }
            body["tools"] = Value::Array(
                request
                    .tools
                    .iter()
                    .map(|tool| {
                        json!({
                            "type":"function","name":tool.name,"description":tool.description,
                            "parameters":tool.parameters,"strict":true
                        })
                    })
                    .collect(),
            );
            body["tool_choice"] = json!(if matches!(mode, BackgroundMode::ToolsAuto) {
                "auto"
            } else {
                "required"
            });
            body["parallel_tool_calls"] = json!(matches!(mode, BackgroundMode::ToolsParallel));
        }
    }
    Ok(body)
}

fn parse_turn(
    body: &Value,
    mode: &crate::providers::llm::BackgroundMode,
) -> Result<crate::providers::llm::BackgroundTurnResult, LlmError> {
    use crate::providers::llm::{BackgroundMode, BackgroundTurn, BackgroundTurnResult, ToolCall};
    if body["status"] != "completed" {
        return Err(LlmError::Rejected);
    }
    let output = body["output"].as_array().ok_or(LlmError::Rejected)?;
    let turn = match mode {
        BackgroundMode::Structured { .. } => BackgroundTurn::Text(output_text(body)?),
        BackgroundMode::Tools | BackgroundMode::ToolsAuto | BackgroundMode::ToolsParallel => {
            let mut calls = Vec::new();
            let mut text_returned = false;
            for item in output {
                match item["type"].as_str() {
                    Some("reasoning") => {
                        // This stateless adapter cannot replay opaque continuation
                        // blocks yet. Reject before any tool dispatch, rather than
                        // silently dropping provider-required native state.
                        if !item["encrypted_content"].is_null() || !item["id"].is_null() {
                            return Err(LlmError::Rejected);
                        }
                    }
                    Some("message") => {
                        text_returned = true;
                    }
                    Some("function_call") => {
                        let id = item["call_id"]
                            .as_str()
                            .filter(|s| !s.is_empty())
                            .ok_or(LlmError::Rejected)?;
                        let name = item["name"]
                            .as_str()
                            .filter(|s| !s.is_empty())
                            .ok_or(LlmError::Rejected)?;
                        let args = item["arguments"].as_str().ok_or(LlmError::Rejected)?;
                        calls.push(ToolCall {
                            id: id.to_owned(),
                            name: name.to_owned(),
                            arguments: serde_json::from_str(args)
                                .unwrap_or_else(|_| Value::String(args.to_owned())),
                        });
                    }
                    _ => return Err(LlmError::Rejected),
                }
            }
            if text_returned {
                if !calls.is_empty() {
                    return Err(LlmError::Rejected);
                }
                // Text-only native turns belong to the Child decision validator.
                BackgroundTurn::Text(output_text(body)?)
            } else {
                BackgroundTurn::ToolCalls(calls)
            }
        }
    };
    let usage = body
        .get("usage")
        .map(|v| crate::providers::llm::BackgroundTokenUsage {
            input_tokens: v["input_tokens"].as_u64(),
            output_tokens: v["output_tokens"].as_u64(),
            reasoning_tokens: v
                .pointer("/output_tokens_details/reasoning_tokens")
                .and_then(Value::as_u64),
            cached_input_tokens: v
                .pointer("/input_tokens_details/cached_tokens")
                .and_then(Value::as_u64),
        });
    Ok(BackgroundTurnResult { turn, usage })
}

/// Strict one-shot Responses path for hierarchical Job turns. Does not reuse remote state.
pub(crate) async fn turn_with_usage(
    base_url: &str,
    model: &str,
    api_key: &str,
    timeout_seconds: u64,
    request: LlmRequest,
    mode: crate::providers::llm::BackgroundMode,
) -> Result<crate::providers::llm::BackgroundTurnResult, LlmError> {
    let url = url::Url::parse(&format!("{}/responses", base_url.trim_end_matches('/')))
        .map_err(|_| LlmError::Failed)?;
    if !matches!(url.scheme(), "https" | "http")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(LlmError::Failed);
    }
    let body = turn_body(model, &request, &mode)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_seconds))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| LlmError::Failed)?;
    let mut send = client.post(url).json(&body);
    if !api_key.is_empty() {
        send = send.bearer_auth(api_key);
    }
    let mut response = send.send().await.map_err(|_| LlmError::Failed)?;
    if !response.status().is_success() {
        return Err(LlmError::Rejected);
    }
    if response
        .content_length()
        .is_some_and(|n| n > MAX_RESPONSE_BYTES as u64)
    {
        return Err(LlmError::Failed);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| LlmError::Failed)? {
        if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(LlmError::Failed);
        }
        bytes.extend(chunk);
    }
    let answer: Value = serde_json::from_slice(&bytes).map_err(|_| LlmError::Rejected)?;
    parse_turn(&answer, &mode)
}

#[cfg(test)]
#[path = "responses_background_tests.rs"]
mod background_tool_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::llm::ToolDefinition;

    use crate::providers::llm::{BackgroundMode, BackgroundTurn, ConfiguredOpenAiLlm};

    fn request() -> LlmRequest {
        LlmRequest {
            messages: LlmRequest::text_turn("Complete the task".into()).messages,
            tools: vec![ToolDefinition {
                name: "submit_result".into(),
                description: "Complete task".into(),
                parameters: json!({"type":"object","properties":{},"additionalProperties":false}),
            }],
        }
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
                assert_eq!(body["tool_choice"], "auto");
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
        let p = ConfiguredOpenAiLlm::build_with_responses("test", &base, "gpt-test", 5, true)
            .unwrap()
            .with_background_capabilities(false, true);
        let answer = p
            .background_turn(request(), BackgroundMode::ToolsAuto)
            .await
            .unwrap();
        assert!(matches!(answer.turn, BackgroundTurn::Text(text) if text == r#"{"steps":[]}"#));
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
                assert_eq!(body["tool_choice"], "auto");
                assert_eq!(body["store"], false);
                assert_eq!(body["background"], false);
                assert_eq!(body["tools"][0]["name"], "submit_result");
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
        let result = turn_with_usage(
            &base,
            "muse-spark-1.3-contributor-free",
            "public",
            5,
            request(),
            BackgroundMode::ToolsAuto,
        )
        .await
        .unwrap();
        assert!(matches!(result.turn, BackgroundTurn::Text(text) if text == r#"{"summary":"ok"}"#));
        server.abort();
    }

    #[tokio::test]
    async fn background_responses_return_optional_usage_without_hidden_reasoning() {
        use axum::{Json, Router, routing::post};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/v1", listener.local_addr().unwrap());
        let app = Router::new().route(
            "/v1/responses",
            post(|| async {
                Json(json!({
                    "status": "completed",
                    "output": [{"type":"message","role":"assistant",
                        "content":[{"type":"output_text","text":r#"{"summary":"ok"}"#}]}],
                    "usage": {
                        "input_tokens": 121,
                        "output_tokens": 37,
                        "output_tokens_details": {"reasoning_tokens": 9},
                        "input_tokens_details": {"cached_tokens": 20}
                    }
                }))
            }),
        );
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let result = turn_with_usage(
            &base,
            "gpt-test",
            "test",
            5,
            request(),
            BackgroundMode::ToolsAuto,
        )
        .await
        .unwrap();
        assert!(matches!(result.turn, BackgroundTurn::Text(text) if text == r#"{"summary":"ok"}"#));
        let usage = result.usage.unwrap();
        assert_eq!(usage.input_tokens, Some(121));
        assert_eq!(usage.output_tokens, Some(37));
        assert_eq!(usage.reasoning_tokens, Some(9));
        assert_eq!(usage.cached_input_tokens, Some(20));
        server.abort();
    }
}
