use std::collections::{BTreeMap, HashMap};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use futures_util::stream::poll_fn;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::{
    providers::llm::{
        ChatMessage, LlmError, LlmEvent, LlmEventStream, LlmOutput, LlmRequest, ToolCall,
        collect_chat_stream,
    },
    services::llm_gateway::GatewayLease,
};

use super::{
    config::LlmGatewayConfig,
    provider::{WireError, apply_lease_headers, check_wire_status, parse_tool_arguments},
};

const DEFAULT_CODEX_INSTRUCTIONS: &str = "You are a helpful assistant.";
const DEFAULT_CODEX_FALLBACK_MODEL: &str = "gpt-5.6-terra";
const CODEX_ORIGINATOR: &str = "codex_cli_rs";
const EFFORT_SUFFIXES: &[&str] = &["none", "minimal", "low", "medium", "high", "xhigh"];

fn responses_url(endpoint_base: &str) -> String {
    let trimmed = endpoint_base.trim_end_matches('/');
    if trimmed.ends_with("/responses") {
        trimmed.to_owned()
    } else {
        format!("{trimmed}/responses")
    }
}

pub(crate) fn normalize_codex_model(raw_model: &str) -> (String, Option<String>) {
    let mut model = raw_model.trim().to_owned();
    if let Some(stripped) = model.strip_suffix("[1m]") {
        model = stripped.to_owned();
    }
    if model != "codex-auto-review"
        && let Some(stripped) = model.strip_suffix("-review")
    {
        model = stripped.to_owned();
    }
    let mut model_effort = None;
    for level in EFFORT_SUFFIXES {
        let suffix = format!("-{level}");
        if let Some(stripped) = model.strip_suffix(&suffix)
            && !stripped.is_empty()
        {
            model = stripped.to_owned();
            model_effort = Some((*level).to_owned());
            break;
        }
    }
    (model, model_effort)
}

fn normalize_tool_parameters(params: &Value) -> Value {
    if let Some(obj) = params.as_object() {
        if obj.get("type").and_then(Value::as_str) == Some("object")
            && !obj.contains_key("properties")
        {
            let mut cloned = obj.clone();
            cloned.insert("properties".to_owned(), json!({}));
            return Value::Object(cloned);
        }
        return params.clone();
    }
    json!({
        "type": "object",
        "properties": {},
    })
}

fn build_responses_body(
    model: &str,
    model_effort: Option<&str>,
    config: &LlmGatewayConfig,
    request: &LlmRequest,
    override_input: Option<Vec<Value>>,
) -> Value {
    let mut system_parts = Vec::new();
    let mut input = Vec::new();

    for msg in &request.messages {
        match msg {
            ChatMessage::System { content } => {
                if !content.trim().is_empty() {
                    system_parts.push(content.trim().to_owned());
                }
            }
            ChatMessage::User { content } => {
                input.push(json!({
                    "type": "message",
                    "role": "user",
                    "content": [{ "type": "input_text", "text": content }],
                }));
            }
            ChatMessage::AssistantText { content } => {
                input.push(json!({
                    "type": "message",
                    "role": "assistant",
                    "content": [{ "type": "output_text", "text": content }],
                }));
            }
            ChatMessage::AssistantToolCalls { calls } => {
                for c in calls {
                    let name = c.name.trim();
                    if name.is_empty() {
                        continue;
                    }
                    let call_id = if c.id.trim().is_empty() {
                        format!("call_{name}")
                    } else {
                        c.id.clone()
                    };
                    let arguments = c
                        .arguments
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| c.arguments.to_string());
                    input.push(json!({
                        "type": "function_call",
                        "call_id": call_id,
                        "name": name,
                        "arguments": arguments,
                    }));
                }
            }
            ChatMessage::ToolResult {
                tool_call_id,
                content,
            } => {
                input.push(json!({
                    "type": "function_call_output",
                    "call_id": tool_call_id,
                    "output": content,
                }));
            }
        }
    }

    if let Some(custom_input) = override_input {
        input = custom_input;
    }
    if input.is_empty() {
        input.push(json!({
            "type": "message",
            "role": "user",
            "content": [{ "type": "input_text", "text": "..." }],
        }));
    }

    let instructions = if system_parts.is_empty() {
        DEFAULT_CODEX_INSTRUCTIONS.to_owned()
    } else {
        system_parts.join("\n\n")
    };

    let mut body = json!({
        "model": model,
        "instructions": instructions,
        "input": input,
        "stream": true,
        "store": false,
    });

    if let Some(effort) = config.reasoning_effort.as_deref().or(model_effort) {
        body["reasoning"] = json!({ "effort": effort });
    }

    if !request.tools.is_empty() {
        let tools: Vec<Value> = request
            .tools
            .iter()
            .filter(|t| !t.name.trim().is_empty())
            .map(|t| {
                json!({
                    "type": "function",
                    "name": t.name,
                    "description": t.description,
                    "parameters": normalize_tool_parameters(&t.parameters),
                })
            })
            .collect();
        if !tools.is_empty() {
            body["tools"] = json!(tools);
        }
    }

    body
}

fn is_codex_model_unsupported_error(status: reqwest::StatusCode, body_text: &str) -> bool {
    if !matches!(status.as_u16(), 400 | 404) {
        return false;
    }
    let lower = body_text.to_ascii_lowercase();
    lower.contains("model_not_found")
        || lower.contains("not supported when using codex")
        || lower.contains("model is not supported")
        || lower.contains("does not exist")
}

async fn send_codex_responses_request(
    http: &reqwest::Client,
    url: &str,
    lease: &GatewayLease,
    mut body: Value,
) -> Result<reqwest::Response, WireError> {
    let has_originator = lease
        .extra_headers
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case("originator"));

    let build_req = |payload: &Value| {
        let mut builder =
            apply_lease_headers(http.post(url).header("Accept", "text/event-stream"), lease);
        if !has_originator {
            builder = builder.header("originator", CODEX_ORIGINATOR);
        }
        builder.json(payload)
    };

    let resp = build_req(&body)
        .send()
        .await
        .map_err(|_| WireError::Other)?;
    let status = resp.status();
    if matches!(status.as_u16(), 400 | 404) {
        let current_model = body
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        if current_model != DEFAULT_CODEX_FALLBACK_MODEL {
            let err_text = resp.text().await.unwrap_or_default();
            if is_codex_model_unsupported_error(status, &err_text) {
                body["model"] = json!(DEFAULT_CODEX_FALLBACK_MODEL);
                let retry_resp = build_req(&body)
                    .send()
                    .await
                    .map_err(|_| WireError::Other)?;
                return check_wire_status(retry_resp);
            }
        }
        return Err(WireError::HttpRejected);
    }
    check_wire_status(resp)
}

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
    open_sse_stream_internal(http, endpoint_base, lease, config, request, None).await
}

async fn open_sse_stream_internal(
    http: &reqwest::Client,
    endpoint_base: &str,
    lease: &GatewayLease,
    config: &LlmGatewayConfig,
    request: &LlmRequest,
    override_input: Option<Vec<Value>>,
) -> Result<LlmEventStream, WireError> {
    let url = responses_url(endpoint_base);
    let (model, model_effort) = normalize_codex_model(&config.model);
    let body = build_responses_body(
        &model,
        model_effort.as_deref(),
        config,
        request,
        override_input,
    );
    let mut resp = send_codex_responses_request(http, &url, lease, body).await?;

    let (tx, mut rx) = mpsc::unbounded_channel::<Result<LlmEvent, LlmError>>();
    tokio::spawn(async move {
        let mut buffer = String::new();
        let mut pending_tools: BTreeMap<u64, (String, String, String)> = BTreeMap::new();
        let mut item_id_to_index: HashMap<String, u64> = HashMap::new();
        let mut next_tool_index: u64 = 0;

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
                if payload == "[DONE]" {
                    if flush_pending_tools(std::mem::take(&mut pending_tools), &tx) {
                        let _ = tx.send(Ok(LlmEvent::Finished));
                    }
                    return;
                }
                let Ok(val) = serde_json::from_str::<Value>(payload) else {
                    continue;
                };

                if let Some(event_type) = val.get("type").and_then(Value::as_str) {
                    match event_type {
                        "response.output_text.delta" => {
                            if let Some(delta) = val.get("delta").and_then(Value::as_str)
                                && !delta.is_empty()
                            {
                                let _ = tx.send(Ok(LlmEvent::TextDelta(delta.to_owned())));
                            }
                        }
                        "response.output_item.added" => {
                            if let Some(item) = val.get("item")
                                && matches!(
                                    item.get("type").and_then(Value::as_str),
                                    Some("function_call" | "custom_tool_call")
                                )
                            {
                                let idx = val
                                    .get("output_index")
                                    .and_then(Value::as_u64)
                                    .unwrap_or_else(|| {
                                        let assigned = next_tool_index;
                                        next_tool_index += 1;
                                        assigned
                                    });
                                next_tool_index = next_tool_index.max(idx + 1);
                                if let Some(item_id) = item.get("id").and_then(Value::as_str)
                                    && !item_id.is_empty()
                                {
                                    item_id_to_index.insert(item_id.to_owned(), idx);
                                }
                                let entry = pending_tools.entry(idx).or_default();
                                if let Some(call_id) = item.get("call_id").and_then(Value::as_str)
                                    && !call_id.is_empty()
                                {
                                    entry.0 = call_id.to_owned();
                                }
                                if let Some(name) = item.get("name").and_then(Value::as_str)
                                    && !name.is_empty()
                                {
                                    entry.1 = name.to_owned();
                                }
                                if let Some(args) = item.get("arguments").and_then(Value::as_str)
                                    && !args.is_empty()
                                {
                                    entry.2.push_str(args);
                                }
                            }
                        }
                        "response.function_call_arguments.delta"
                        | "response.custom_tool_call_input.delta" => {
                            if let Some(delta) = val.get("delta").and_then(Value::as_str)
                                && !delta.is_empty()
                            {
                                let idx = val
                                    .get("item_id")
                                    .and_then(Value::as_str)
                                    .and_then(|id| item_id_to_index.get(id).copied())
                                    .or_else(|| val.get("output_index").and_then(Value::as_u64))
                                    .unwrap_or(0);
                                pending_tools.entry(idx).or_default().2.push_str(delta);
                            }
                        }
                        "response.output_item.done" => {
                            if let Some(item) = val.get("item")
                                && matches!(
                                    item.get("type").and_then(Value::as_str),
                                    Some("function_call" | "custom_tool_call")
                                )
                            {
                                let idx = item
                                    .get("id")
                                    .and_then(Value::as_str)
                                    .and_then(|id| item_id_to_index.get(id).copied())
                                    .or_else(|| val.get("output_index").and_then(Value::as_u64))
                                    .unwrap_or(0);
                                let entry = pending_tools.entry(idx).or_default();
                                if entry.0.is_empty()
                                    && let Some(call_id) =
                                        item.get("call_id").and_then(Value::as_str)
                                {
                                    entry.0 = call_id.to_owned();
                                }
                                if entry.1.is_empty()
                                    && let Some(name) = item.get("name").and_then(Value::as_str)
                                {
                                    entry.1 = name.to_owned();
                                }
                                if entry.2.is_empty()
                                    && let Some(args) =
                                        item.get("arguments").and_then(Value::as_str)
                                    && !args.is_empty()
                                {
                                    entry.2 = args.to_owned();
                                }
                            }
                        }
                        "response.completed" | "response.done" => {
                            if flush_pending_tools(std::mem::take(&mut pending_tools), &tx) {
                                let _ = tx.send(Ok(LlmEvent::Finished));
                            }
                            return;
                        }
                        "error" | "response.failed" => {
                            let _ = tx.send(Err(LlmError::Failed));
                            return;
                        }
                        _ => {}
                    }
                    continue;
                }

                // Fallback support for standard Chat Completions SSE chunks if returned by a custom proxy
                if let Some(choices) = val.get("choices").and_then(Value::as_array) {
                    for choice in choices {
                        if let Some(delta) = choice.get("delta")
                            && let Some(content) = delta.get("content").and_then(Value::as_str)
                            && !content.is_empty()
                        {
                            let _ = tx.send(Ok(LlmEvent::TextDelta(content.to_owned())));
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
    let data_url = format!(
        "data:{mime_type};base64,{}",
        BASE64_STANDARD.encode(image_bytes)
    );
    let vision_input = vec![json!({
        "type": "message",
        "role": "user",
        "content": [
            { "type": "input_text", "text": prompt },
            { "type": "input_image", "image_url": data_url, "detail": "auto" }
        ]
    })];
    let dummy_req = LlmRequest::text_turn(prompt.to_owned());
    let stream = open_sse_stream_internal(
        http,
        endpoint_base,
        lease,
        config,
        &dummy_req,
        Some(vision_input),
    )
    .await?;
    let out = collect_chat_stream(stream)
        .await
        .map_err(|_| WireError::Other)?;
    if let LlmOutput::Text(ref t) = out
        && t.trim().is_empty()
    {
        return Err(WireError::Other);
    }
    Ok(out)
}
