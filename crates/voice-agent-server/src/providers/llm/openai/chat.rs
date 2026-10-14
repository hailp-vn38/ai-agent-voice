//! One-shot strict Background Chat Completions. Voice keeps its streaming adapter.
use crate::providers::llm::{
    BackgroundMode, BackgroundTokenUsage, BackgroundTurn, BackgroundTurnResult, ChatMessage,
    LlmError, LlmRequest, ToolCall,
};
use serde_json::{Value, json};
use std::time::Duration;

const MAX_RESPONSE_BYTES: usize = 256 * 1024;

fn request_body(
    model: &str,
    request: &LlmRequest,
    mode: &BackgroundMode,
) -> Result<Value, LlmError> {
    let mut messages = Vec::with_capacity(request.messages.len());
    for message in &request.messages {
        messages.push(match message {
            ChatMessage::System { content } => json!({"role":"system","content":content}),
            ChatMessage::User { content } => json!({"role":"user","content":content}),
            ChatMessage::AssistantText { content } => json!({"role":"assistant","content":content}),
            ChatMessage::AssistantToolCalls { calls } => json!({"role":"assistant","tool_calls":
                calls.iter().map(|call| json!({"id":call.id,"type":"function",
                    "function":{"name":call.name,"arguments":call.arguments.to_string()}})).collect::<Vec<_>>()}),
            ChatMessage::ToolResult { tool_call_id, content } => json!({
                "role":"tool","tool_call_id":tool_call_id,"content":content
            }),
        });
    }
    if messages.is_empty() {
        return Err(LlmError::Rejected);
    }
    let mut body = json!({"model":model,"messages":messages,"stream":false});
    match mode {
        BackgroundMode::Structured { name, schema } => {
            if !request.tools.is_empty() {
                return Err(LlmError::Rejected);
            }
            body["response_format"] = json!({
                "type":"json_schema","json_schema":{"name":name,"strict":true,"schema":schema}
            });
        }
        BackgroundMode::Tools => {
            if request.tools.is_empty() {
                return Err(LlmError::Rejected);
            }
            body["tools"] = Value::Array(request.tools.iter().map(|tool| json!({
                "type":"function","function":{"name":tool.name,"description":tool.description,
                    "parameters":tool.parameters,"strict":true}
            })).collect());
            body["tool_choice"] = json!("required");
            body["parallel_tool_calls"] = json!(false);
        }
    }
    Ok(body)
}

fn parse_response(body: &Value, mode: &BackgroundMode) -> Result<BackgroundTurnResult, LlmError> {
    let choice = body.pointer("/choices/0").ok_or(LlmError::Rejected)?;
    if choice["finish_reason"] == "length" || choice["finish_reason"] == "content_filter" {
        return Err(LlmError::Rejected);
    }
    let message = &choice["message"];
    if !message["refusal"].is_null() {
        return Err(LlmError::Rejected);
    }
    let turn = match mode {
        BackgroundMode::Structured { .. } => {
            if message["tool_calls"].is_array() {
                return Err(LlmError::Rejected);
            }
            let content = message["content"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or(LlmError::Rejected)?;
            BackgroundTurn::Text(content.to_owned())
        }
        BackgroundMode::Tools => {
            let calls = message["tool_calls"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            let mut parsed = Vec::with_capacity(calls.len());
            for call in calls {
                if call["type"] != "function" {
                    return Err(LlmError::Rejected);
                }
                let id = call["id"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or(LlmError::Rejected)?;
                let name = call["function"]["name"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or(LlmError::Rejected)?;
                let args = call["function"]["arguments"]
                    .as_str()
                    .ok_or(LlmError::Rejected)?;
                let arguments: Value =
                    serde_json::from_str(args).unwrap_or_else(|_| Value::String(args.to_owned()));
                parsed.push(ToolCall {
                    id: id.to_owned(),
                    name: name.to_owned(),
                    arguments,
                });
            }
            BackgroundTurn::ToolCalls(parsed)
        }
    };
    let usage = body.get("usage").map(|v| BackgroundTokenUsage {
        input_tokens: v["prompt_tokens"].as_u64(),
        output_tokens: v["completion_tokens"].as_u64(),
        reasoning_tokens: v
            .pointer("/completion_tokens_details/reasoning_tokens")
            .and_then(Value::as_u64),
        cached_input_tokens: v
            .pointer("/prompt_tokens_details/cached_tokens")
            .and_then(Value::as_u64),
    });
    Ok(BackgroundTurnResult { turn, usage })
}

pub(crate) async fn turn_with_usage(
    base_url: &str,
    model: &str,
    api_key: &str,
    timeout_seconds: u64,
    request: LlmRequest,
    mode: BackgroundMode,
    extra_headers: &[(String, String)],
) -> Result<BackgroundTurnResult, LlmError> {
    let url = url::Url::parse(&format!(
        "{}/chat/completions",
        base_url.trim_end_matches('/')
    ))
    .map_err(|_| LlmError::Failed)?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(LlmError::Failed);
    }
    let body = request_body(model, &request, &mode)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_seconds))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| LlmError::Failed)?;
    let mut send = client.post(url).json(&body);
    if !api_key.is_empty() {
        send = send.bearer_auth(api_key);
    }
    for (key, value) in extra_headers {
        send = send.header(key.as_str(), value.as_str());
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
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| LlmError::Rejected)?;
    parse_response(&value, &mode)
}

#[cfg(test)]
#[path = "chat_tests.rs"]
mod tests;
