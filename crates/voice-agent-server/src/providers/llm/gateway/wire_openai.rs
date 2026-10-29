use std::collections::BTreeMap;

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use futures_util::stream::poll_fn;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::{
    providers::llm::{
        ChatMessage, LlmError, LlmEvent, LlmEventStream, LlmOutput, LlmRequest, ToolCall,
    },
    services::llm_gateway::GatewayLease,
};

use super::{
    config::LlmGatewayConfig,
    provider::{WireError, apply_lease_headers, check_wire_status, parse_tool_arguments},
};

fn flush_pending_tools(
    pending_tools: BTreeMap<u64, (String, String, String)>,
    tx: &mpsc::UnboundedSender<Result<LlmEvent, LlmError>>,
) -> bool {
    for (_idx, (id, name, args)) in pending_tools {
        if !name.is_empty() {
            let Ok(arguments) = parse_tool_arguments(&args) else {
                let _ = tx.send(Err(LlmError::Failed));
                return false;
            };
            let _ = tx.send(Ok(LlmEvent::ToolCall(ToolCall {
                id: if id.is_empty() {
                    format!("call_{name}")
                } else {
                    id
                },
                name,
                arguments,
            })));
        }
    }
    true
}

pub(crate) async fn open_sse_stream(
    http: &reqwest::Client,
    endpoint_base: &str,
    lease: &GatewayLease,
    config: &LlmGatewayConfig,
    request: &LlmRequest,
) -> Result<LlmEventStream, WireError> {
    let url = format!("{}/chat/completions", endpoint_base.trim_end_matches('/'));
    let messages: Vec<Value> = request
        .messages
        .iter()
        .map(|msg| match msg {
            ChatMessage::System { content } => json!({
                "role": "system",
                "content": content,
            }),
            ChatMessage::User { content } => json!({
                "role": "user",
                "content": content,
            }),
            ChatMessage::AssistantText { content } => json!({
                "role": "assistant",
                "content": content,
            }),
            ChatMessage::AssistantToolCalls { calls } => {
                let tool_calls: Vec<Value> = calls
                    .iter()
                    .map(|c| {
                        json!({
                            "id": c.id,
                            "type": "function",
                            "function": {
                                "name": c.name,
                                "arguments": c.arguments.to_string(),
                            }
                        })
                    })
                    .collect();
                json!({
                    "role": "assistant",
                    "content": Value::Null,
                    "tool_calls": tool_calls,
                })
            }
            ChatMessage::ToolResult {
                tool_call_id,
                content,
            } => json!({
                "role": "tool",
                "tool_call_id": tool_call_id,
                "content": content,
            }),
        })
        .collect();

    let mut body = json!({
        "model": config.model,
        "messages": messages,
        "stream": true,
    });
    if let Some(max_tokens) = config.max_tokens {
        body["max_tokens"] = json!(max_tokens);
    }
    if let Some(pct) = config.temperature_percent {
        body["temperature"] = json!((f32::from(pct) / 100.0).clamp(0.0, 2.0));
    }
    if let Some(ref effort) = config.reasoning_effort {
        body["reasoning_effort"] = json!(effort);
    }
    if !request.tools.is_empty() {
        let tools: Vec<Value> = request
            .tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.parameters,
                    }
                })
            })
            .collect();
        body["tools"] = json!(tools);
    }

    let req_builder =
        apply_lease_headers(http.post(&url).header("Accept", "text/event-stream"), lease);
    let resp = req_builder
        .json(&body)
        .send()
        .await
        .map_err(|_| WireError::Other)?;
    let mut resp = check_wire_status(resp)?;

    let (tx, mut rx) = mpsc::unbounded_channel::<Result<LlmEvent, LlmError>>();
    tokio::spawn(async move {
        let mut buffer = String::new();
        let mut pending_tools: BTreeMap<u64, (String, String, String)> = BTreeMap::new();

        loop {
            let chunk = match resp.chunk().await {
                Ok(Some(c)) => c,
                Ok(None) => break,
                Err(_) => {
                    let _ = tx.send(Err(LlmError::Failed));
                    return;
                }
            };
            let Ok(text) = std::str::from_utf8(&chunk) else {
                let _ = tx.send(Err(LlmError::Failed));
                return;
            };
            buffer.push_str(text);

            while let Some(newline_pos) = buffer.find('\n') {
                let line = buffer[..newline_pos].trim().to_owned();
                buffer.drain(..=newline_pos);
                if line.is_empty() || line.starts_with(':') {
                    continue;
                }
                let Some(data) = line.strip_prefix("data:") else {
                    continue;
                };
                let payload = data.trim();
                if payload == "[DONE]" {
                    if flush_pending_tools(std::mem::take(&mut pending_tools), &tx) {
                        let _ = tx.send(Ok(LlmEvent::Finished));
                    }
                    return;
                }
                let Ok(val) = serde_json::from_str::<Value>(payload) else {
                    continue;
                };
                let Some(choices) = val.get("choices").and_then(Value::as_array) else {
                    continue;
                };
                for choice in choices {
                    if let Some(delta) = choice.get("delta") {
                        if let Some(content) = delta.get("content").and_then(Value::as_str)
                            && !content.is_empty()
                        {
                            let _ = tx.send(Ok(LlmEvent::TextDelta(content.to_owned())));
                        }
                        if let Some(tc_arr) = delta.get("tool_calls").and_then(Value::as_array) {
                            for tc in tc_arr {
                                let idx = tc.get("index").and_then(Value::as_u64).unwrap_or(0);
                                let entry = pending_tools.entry(idx).or_default();
                                if let Some(id) = tc.get("id").and_then(Value::as_str)
                                    && !id.is_empty()
                                {
                                    entry.0 = id.to_owned();
                                }
                                if let Some(func) = tc.get("function") {
                                    if let Some(name) = func.get("name").and_then(Value::as_str)
                                        && !name.is_empty()
                                    {
                                        entry.1.push_str(name);
                                    }
                                    if let Some(args) =
                                        func.get("arguments").and_then(Value::as_str)
                                    {
                                        entry.2.push_str(args);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        if flush_pending_tools(pending_tools, &tx) {
            let _ = tx.send(Ok(LlmEvent::Finished));
        }
    });

    Ok(Box::pin(poll_fn(move |cx| rx.poll_recv(cx))))
}

pub(crate) async fn execute_vision(
    http: &reqwest::Client,
    endpoint_base: &str,
    lease: &GatewayLease,
    config: &LlmGatewayConfig,
    prompt: &str,
    image_bytes: &[u8],
    mime_type: &str,
) -> Result<LlmOutput, WireError> {
    let url = format!("{}/chat/completions", endpoint_base.trim_end_matches('/'));
    let data_url = format!(
        "data:{mime_type};base64,{}",
        BASE64_STANDARD.encode(image_bytes)
    );
    let body = json!({
        "model": config.model,
        "messages": [
            {
                "role": "user",
                "content": [
                    { "type": "text", "text": prompt },
                    { "type": "image_url", "image_url": { "url": data_url } }
                ]
            }
        ],
        "max_tokens": config.max_tokens.unwrap_or(1024),
        "stream": false,
    });

    let req_builder = apply_lease_headers(http.post(&url), lease);
    let resp = req_builder
        .json(&body)
        .send()
        .await
        .map_err(|_| WireError::Other)?;
    let resp = check_wire_status(resp)?;
    let val: Value = resp.json().await.map_err(|_| WireError::Other)?;
    let text = val
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|arr| arr.first())
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_owned();
    if text.is_empty() {
        return Err(WireError::Other);
    }
    Ok(LlmOutput::Text(text))
}
