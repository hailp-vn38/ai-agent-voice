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

fn emit_anthropic_tool_call(
    idx: u64,
    id: String,
    name: String,
    args: &str,
    tx: &mpsc::UnboundedSender<Result<LlmEvent, LlmError>>,
) -> bool {
    if name.is_empty() {
        return true;
    }
    let Ok(arguments) = parse_tool_arguments(args) else {
        let _ = tx.send(Err(LlmError::Failed));
        return false;
    };
    let _ = tx.send(Ok(LlmEvent::ToolCall(ToolCall {
        id: if id.is_empty() {
            format!("toolu_{idx}")
        } else {
            id
        },
        name,
        arguments,
    })));
    true
}

pub(crate) async fn open_sse_stream(
    http: &reqwest::Client,
    endpoint_base: &str,
    lease: &GatewayLease,
    config: &LlmGatewayConfig,
    request: &LlmRequest,
) -> Result<LlmEventStream, WireError> {
    let url = format!("{}/messages", endpoint_base.trim_end_matches('/'));
    let mut system_prompts: Vec<&str> = Vec::new();
    let mut messages: Vec<Value> = Vec::new();

    for msg in &request.messages {
        match msg {
            ChatMessage::System { content } => {
                if !content.trim().is_empty() {
                    system_prompts.push(content.as_str());
                }
            }
            ChatMessage::User { content } => {
                messages.push(json!({
                    "role": "user",
                    "content": content,
                }));
            }
            ChatMessage::AssistantText { content } => {
                messages.push(json!({
                    "role": "assistant",
                    "content": content,
                }));
            }
            ChatMessage::AssistantToolCalls { calls } => {
                let mut blocks: Vec<Value> = calls
                    .iter()
                    .map(|c| {
                        json!({
                            "type": "tool_use",
                            "id": c.id,
                            "name": c.name,
                            "input": c.arguments,
                        })
                    })
                    .collect();
                if let Some(last) = messages.last_mut()
                    && last.get("role").and_then(Value::as_str) == Some("assistant")
                {
                    if let Some(existing_str) = last
                        .get("content")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                    {
                        let mut merged = Vec::with_capacity(blocks.len() + 1);
                        if !existing_str.trim().is_empty() {
                            merged.push(json!({
                                "type": "text",
                                "text": existing_str,
                            }));
                        }
                        merged.append(&mut blocks);
                        last["content"] = Value::Array(merged);
                        continue;
                    }
                    if let Some(arr) = last.get_mut("content").and_then(Value::as_array_mut) {
                        arr.append(&mut blocks);
                        continue;
                    }
                }
                messages.push(json!({
                    "role": "assistant",
                    "content": blocks,
                }));
            }
            ChatMessage::ToolResult {
                tool_call_id,
                content,
            } => {
                let block = json!({
                    "type": "tool_result",
                    "tool_use_id": tool_call_id,
                    "content": content,
                });
                if let Some(last) = messages.last_mut()
                    && last.get("role").and_then(Value::as_str) == Some("user")
                    && let Some(arr) = last.get_mut("content").and_then(Value::as_array_mut)
                {
                    arr.push(block);
                } else {
                    messages.push(json!({
                        "role": "user",
                        "content": [block],
                    }));
                }
            }
        }
    }

    let mut body = json!({
        "model": config.model,
        "max_tokens": config.max_tokens.unwrap_or(4096),
        "messages": messages,
        "stream": true,
    });
    if !system_prompts.is_empty() {
        body["system"] = json!(system_prompts.join("\n\n"));
    }
    if let Some(pct) = config.temperature_percent {
        body["temperature"] = json!((f32::from(pct) / 100.0).clamp(0.0, 1.0));
    }
    if !request.tools.is_empty() {
        let tools: Vec<Value> = request
            .tools
            .iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.parameters,
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
        let mut active_tools: BTreeMap<u64, (String, String, String)> = BTreeMap::new();

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
                if line.is_empty() || line.starts_with(':') || line.starts_with("event:") {
                    continue;
                }
                let Some(data) = line.strip_prefix("data:") else {
                    continue;
                };
                let payload = data.trim();
                let Ok(val) = serde_json::from_str::<Value>(payload) else {
                    continue;
                };
                let Some(event_type) = val.get("type").and_then(Value::as_str) else {
                    continue;
                };
                match event_type {
                    "content_block_start" => {
                        let idx = val.get("index").and_then(Value::as_u64).unwrap_or(0);
                        if let Some(block) = val.get("content_block")
                            && block.get("type").and_then(Value::as_str) == Some("tool_use")
                        {
                            let id = block
                                .get("id")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_owned();
                            let name = block
                                .get("name")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_owned();
                            let initial_args = match block.get("input") {
                                Some(Value::Object(map)) if !map.is_empty() => {
                                    serde_json::to_string(map).unwrap_or_default()
                                }
                                _ => String::new(),
                            };
                            active_tools.insert(idx, (id, name, initial_args));
                        }
                    }
                    "content_block_delta" => {
                        let idx = val.get("index").and_then(Value::as_u64).unwrap_or(0);
                        if let Some(delta) = val.get("delta") {
                            match delta.get("type").and_then(Value::as_str) {
                                Some("text_delta") => {
                                    if let Some(txt) = delta.get("text").and_then(Value::as_str)
                                        && !txt.is_empty()
                                    {
                                        let _ = tx.send(Ok(LlmEvent::TextDelta(txt.to_owned())));
                                    }
                                }
                                Some("input_json_delta") => {
                                    if let Some(partial) =
                                        delta.get("partial_json").and_then(Value::as_str)
                                        && let Some(entry) = active_tools.get_mut(&idx)
                                    {
                                        entry.2.push_str(partial);
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    "content_block_stop" => {
                        let idx = val.get("index").and_then(Value::as_u64).unwrap_or(0);
                        if let Some((id, name, args)) = active_tools.remove(&idx)
                            && !emit_anthropic_tool_call(idx, id, name, &args, &tx)
                        {
                            return;
                        }
                    }
                    "message_stop" => {
                        for (idx, (id, name, args)) in std::mem::take(&mut active_tools) {
                            if !emit_anthropic_tool_call(idx, id, name, &args, &tx) {
                                return;
                            }
                        }
                        let _ = tx.send(Ok(LlmEvent::Finished));
                        return;
                    }
                    _ => {}
                }
            }
        }

        for (idx, (id, name, args)) in active_tools {
            if !emit_anthropic_tool_call(idx, id, name, &args, &tx) {
                return;
            }
        }
        let _ = tx.send(Ok(LlmEvent::Finished));
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
    let url = format!("{}/messages", endpoint_base.trim_end_matches('/'));
    let body = json!({
        "model": config.model,
        "max_tokens": config.max_tokens.unwrap_or(1024),
        "messages": [
            {
                "role": "user",
                "content": [
                    {
                        "type": "image",
                        "source": {
                            "type": "base64",
                            "media_type": mime_type,
                            "data": BASE64_STANDARD.encode(image_bytes),
                        }
                    },
                    {
                        "type": "text",
                        "text": prompt,
                    }
                ]
            }
        ],
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
        .get("content")
        .and_then(Value::as_array)
        .and_then(|arr| {
            arr.iter().find_map(|b| {
                (b.get("type").and_then(Value::as_str) == Some("text"))
                    .then(|| b.get("text").and_then(Value::as_str))
                    .flatten()
            })
        })
        .unwrap_or("")
        .trim()
        .to_owned();
    if text.is_empty() {
        return Err(WireError::Other);
    }
    Ok(LlmOutput::Text(text))
}
