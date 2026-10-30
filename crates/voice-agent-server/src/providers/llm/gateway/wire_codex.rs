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
    provider::{
        ErrorDetailSlot, WireError, apply_lease_headers, drain_next_sse_line,
        extract_sse_error_detail, new_error_detail_slot, parse_tool_arguments, record_wire_detail,
        summarize_error_body,
    },
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

async fn check_codex_response_status(
    resp: reqwest::Response,
    account_key: &str,
    model: &str,
    err_slot: &ErrorDetailSlot,
) -> Result<reqwest::Response, WireError> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let retry_after = resp
        .headers()
        .get("retry-after")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.trim().parse::<u64>().ok());
    let body_text = resp.text().await.unwrap_or_default();
    let summary = summarize_error_body(&body_text);
    record_wire_detail(
        err_slot,
        format!(
            "Codex HTTP {} on account '{}' (model '{}'): {}",
            status.as_u16(),
            account_key,
            model,
            summary
        ),
    );
    if status.as_u16() == 429 {
        return Err(WireError::RateLimited {
            retry_after_secs: retry_after,
        });
    }
    if matches!(status.as_u16(), 401 | 403) {
        return Err(WireError::Unauthorized);
    }
    if matches!(status.as_u16(), 408 | 504) {
        return Err(WireError::Other);
    }
    Err(WireError::HttpRejected)
}

async fn send_codex_responses_request(
    http: &reqwest::Client,
    url: &str,
    lease: &GatewayLease,
    mut body: Value,
    err_slot: &ErrorDetailSlot,
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

    let current_model = body
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let resp = build_req(&body).send().await.map_err(|e| {
        record_wire_detail(
            err_slot,
            format!(
                "Codex HTTP request failed on account '{}' (model '{}', timeout={}, connect={})",
                lease.account_key,
                current_model,
                e.is_timeout(),
                e.is_connect()
            ),
        );
        WireError::Other
    })?;
    let status = resp.status();
    if matches!(status.as_u16(), 400 | 404) {
        let err_text = resp.text().await.unwrap_or_default();
        if current_model != DEFAULT_CODEX_FALLBACK_MODEL
            && is_codex_model_unsupported_error(status, &err_text)
        {
            body["model"] = json!(DEFAULT_CODEX_FALLBACK_MODEL);
            let retry_resp = build_req(&body).send().await.map_err(|e| {
                record_wire_detail(
                    err_slot,
                    format!(
                        "Codex fallback HTTP request failed on account '{}' (model '{}', timeout={}, connect={})",
                        lease.account_key,
                        DEFAULT_CODEX_FALLBACK_MODEL,
                        e.is_timeout(),
                        e.is_connect()
                    ),
                );
                WireError::Other
            })?;
            return check_codex_response_status(
                retry_resp,
                &lease.account_key,
                DEFAULT_CODEX_FALLBACK_MODEL,
                err_slot,
            )
            .await;
        }
        record_wire_detail(
            err_slot,
            format!(
                "Codex HTTP {} on account '{}' (model '{}'): {}",
                status.as_u16(),
                lease.account_key,
                current_model,
                summarize_error_body(&err_text)
            ),
        );
        return Err(WireError::HttpRejected);
    }
    check_codex_response_status(resp, &lease.account_key, &current_model, err_slot).await
}

fn flush_pending_tools(
    pending_tools: BTreeMap<u64, (String, String, String)>,
    tx: &mpsc::UnboundedSender<Result<LlmEvent, LlmError>>,
    account_key: &str,
    err_slot: &ErrorDetailSlot,
) -> bool {
    for (_idx, (id, name, args)) in pending_tools {
        if !name.is_empty() {
            let Ok(arguments) = parse_tool_arguments(&args) else {
                record_wire_detail(
                    err_slot,
                    format!(
                        "Codex tool '{name}' returned malformed JSON arguments on account '{account_key}'"
                    ),
                );
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

fn extract_message_item_text(item: &Value) -> String {
    let mut out = String::new();
    if let Some(content_arr) = item.get("content").and_then(Value::as_array) {
        for part in content_arr {
            let part_type = part.get("type").and_then(Value::as_str).unwrap_or("");
            if matches!(part_type, "output_text" | "text" | "")
                && let Some(t) = part.get("text").and_then(Value::as_str)
                && !t.is_empty()
            {
                out.push_str(t);
            }
        }
    } else if let Some(t) = item.get("text").and_then(Value::as_str) {
        out.push_str(t);
    }
    out
}

pub(crate) async fn open_sse_stream_with_detail(
    http: &reqwest::Client,
    endpoint_base: &str,
    lease: &GatewayLease,
    config: &LlmGatewayConfig,
    request: &LlmRequest,
    err_slot: ErrorDetailSlot,
) -> Result<LlmEventStream, WireError> {
    open_sse_stream_internal(
        http,
        endpoint_base,
        lease,
        config,
        request,
        None,
        err_slot,
    )
    .await
}

async fn open_sse_stream_internal(
    http: &reqwest::Client,
    endpoint_base: &str,
    lease: &GatewayLease,
    config: &LlmGatewayConfig,
    request: &LlmRequest,
    override_input: Option<Vec<Value>>,
    err_slot: ErrorDetailSlot,
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
    let mut resp = send_codex_responses_request(http, &url, lease, body, &err_slot).await?;
    let account_key = lease.account_key.clone();

    let (tx, mut rx) = mpsc::unbounded_channel::<Result<LlmEvent, LlmError>>();
    tokio::spawn(async move {
        let mut raw_buf: Vec<u8> = Vec::new();
        let mut pending_tools: BTreeMap<u64, (String, String, String)> = BTreeMap::new();
        let mut item_id_to_index: HashMap<String, u64> = HashMap::new();
        let mut next_tool_index: u64 = 0;
        let mut emitted_text = false;
        let mut seen_events: Vec<String> = Vec::new();

        loop {
            let chunk_opt = match resp.chunk().await {
                Ok(Some(c)) => Some(c),
                Ok(None) => None,
                Err(e) => {
                    record_wire_detail(
                        &err_slot,
                        format!(
                            "Codex SSE stream read failed on account '{}' (model '{}', timeout={}, events_seen={:?})",
                            account_key,
                            model,
                            e.is_timeout(),
                            seen_events
                        ),
                    );
                    let _ = tx.send(Err(LlmError::Failed));
                    return;
                }
            };
            let is_eof = chunk_opt.is_none();
            if let Some(chunk) = chunk_opt {
                raw_buf.extend_from_slice(&chunk);
            } else if !raw_buf.is_empty() && !raw_buf.ends_with(b"\n") {
                raw_buf.push(b'\n');
            }

            while let Some(line) = drain_next_sse_line(&mut raw_buf) {
                if line.is_empty() || line.starts_with(':') || line.starts_with("event:") {
                    continue;
                }
                let Some(data) = line.strip_prefix("data:") else {
                    continue;
                };
                let payload = data.trim();
                if payload == "[DONE]" {
                    if !emitted_text && pending_tools.is_empty() {
                        record_wire_detail(
                            &err_slot,
                            format!(
                                "Codex SSE finished at [DONE] with empty output on account '{}' (model '{}', events_seen={:?})",
                                account_key, model, seen_events
                            ),
                        );
                    }
                    if flush_pending_tools(
                        std::mem::take(&mut pending_tools),
                        &tx,
                        &account_key,
                        &err_slot,
                    ) {
                        let _ = tx.send(Ok(LlmEvent::Finished));
                    }
                    return;
                }
                let Ok(val) = serde_json::from_str::<Value>(payload) else {
                    continue;
                };

                if let Some(event_type) = val.get("type").and_then(Value::as_str) {
                    if seen_events.len() < 16 && !seen_events.iter().any(|s| s == event_type) {
                        seen_events.push(event_type.to_owned());
                    }
                    match event_type {
                        "response.output_text.delta" => {
                            if let Some(delta) = val.get("delta").and_then(Value::as_str)
                                && !delta.is_empty()
                            {
                                emitted_text = true;
                                let _ = tx.send(Ok(LlmEvent::TextDelta(delta.to_owned())));
                            }
                        }
                        "response.output_text.done" => {
                            if !emitted_text
                                && let Some(text) = val.get("text").and_then(Value::as_str)
                                && !text.is_empty()
                            {
                                emitted_text = true;
                                let _ = tx.send(Ok(LlmEvent::TextDelta(text.to_owned())));
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
                            if let Some(item) = val.get("item") {
                                let item_type = item.get("type").and_then(Value::as_str);
                                if matches!(item_type, Some("function_call" | "custom_tool_call")) {
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
                                } else if item_type == Some("message") && !emitted_text {
                                    let fallback_text = extract_message_item_text(item);
                                    if !fallback_text.is_empty() {
                                        emitted_text = true;
                                        let _ = tx.send(Ok(LlmEvent::TextDelta(fallback_text)));
                                    }
                                }
                            }
                        }
                        "response.completed" | "response.done" => {
                            let resp_obj = val.get("response");
                            let resp_status = resp_obj
                                .and_then(|r| r.get("status"))
                                .and_then(Value::as_str)
                                .unwrap_or("completed");
                            if resp_status == "failed"
                                || resp_obj
                                    .and_then(|r| r.get("error"))
                                    .is_some_and(|e| !e.is_null())
                            {
                                let detail = extract_sse_error_detail(&val)
                                    .unwrap_or_else(|| format!("response status='{resp_status}'"));
                                record_wire_detail(
                                    &err_slot,
                                    format!(
                                        "Codex {event_type} reported failure on account '{}' (model '{}'): {}",
                                        account_key, model, detail
                                    ),
                                );
                                let _ = tx.send(Err(LlmError::Failed));
                                return;
                            }
                            let mut output_types = Vec::new();
                            if let Some(output_arr) = resp_obj
                                .and_then(|r| r.get("output"))
                                .and_then(Value::as_array)
                            {
                                for (idx_pos, item) in output_arr.iter().enumerate() {
                                    let itype = item
                                        .get("type")
                                        .and_then(Value::as_str)
                                        .unwrap_or("unknown");
                                    output_types.push(itype.to_owned());
                                    if itype == "message" && !emitted_text {
                                        let fallback_text = extract_message_item_text(item);
                                        if !fallback_text.is_empty() {
                                            emitted_text = true;
                                            let _ =
                                                tx.send(Ok(LlmEvent::TextDelta(fallback_text)));
                                        }
                                    } else if matches!(itype, "function_call" | "custom_tool_call")
                                        && pending_tools.is_empty()
                                    {
                                        let call_id = item
                                            .get("call_id")
                                            .and_then(Value::as_str)
                                            .unwrap_or("")
                                            .to_owned();
                                        let name = item
                                            .get("name")
                                            .and_then(Value::as_str)
                                            .unwrap_or("")
                                            .to_owned();
                                        let args = item
                                            .get("arguments")
                                            .and_then(Value::as_str)
                                            .unwrap_or("")
                                            .to_owned();
                                        if !name.is_empty() {
                                            pending_tools
                                                .insert(idx_pos as u64, (call_id, name, args));
                                        }
                                    }
                                }
                            }
                            if !emitted_text && pending_tools.is_empty() {
                                let inc = extract_sse_error_detail(&val);
                                record_wire_detail(
                                    &err_slot,
                                    format!(
                                        "Codex {event_type} produced no text or tool_calls on account '{}' (model '{}', status='{}', output_items={:?}, detail={:?})",
                                        account_key, model, resp_status, output_types, inc
                                    ),
                                );
                            }
                            if flush_pending_tools(
                                std::mem::take(&mut pending_tools),
                                &tx,
                                &account_key,
                                &err_slot,
                            ) {
                                let _ = tx.send(Ok(LlmEvent::Finished));
                            }
                            return;
                        }
                        "response.incomplete" => {
                            let detail = extract_sse_error_detail(&val)
                                .unwrap_or_else(|| "response.incomplete".to_owned());
                            record_wire_detail(
                                &err_slot,
                                format!(
                                    "Codex response.incomplete on account '{}' (model '{}'): {}",
                                    account_key, model, detail
                                ),
                            );
                            if !emitted_text && pending_tools.is_empty() {
                                let _ = tx.send(Err(LlmError::Failed));
                                return;
                            }
                            if flush_pending_tools(
                                std::mem::take(&mut pending_tools),
                                &tx,
                                &account_key,
                                &err_slot,
                            ) {
                                let _ = tx.send(Ok(LlmEvent::Finished));
                            }
                            return;
                        }
                        "error" | "response.failed" => {
                            let detail = extract_sse_error_detail(&val)
                                .unwrap_or_else(|| summarize_error_body(payload));
                            record_wire_detail(
                                &err_slot,
                                format!(
                                    "Codex SSE {event_type} on account '{}' (model '{}'): {}",
                                    account_key, model, detail
                                ),
                            );
                            let _ = tx.send(Err(LlmError::Failed));
                            return;
                        }
                        _ => {}
                    }
                    continue;
                }

                if val.get("error").is_some_and(|e| !e.is_null()) {
                    let detail = extract_sse_error_detail(&val)
                        .unwrap_or_else(|| summarize_error_body(payload));
                    record_wire_detail(
                        &err_slot,
                        format!(
                            "Codex SSE error payload on account '{}' (model '{}'): {}",
                            account_key, model, detail
                        ),
                    );
                    let _ = tx.send(Err(LlmError::Failed));
                    return;
                }

                // Fallback support for standard Chat Completions SSE chunks if returned by a custom proxy
                if let Some(choices) = val.get("choices").and_then(Value::as_array) {
                    for choice in choices {
                        if let Some(delta) = choice.get("delta")
                            && let Some(content) = delta.get("content").and_then(Value::as_str)
                            && !content.is_empty()
                        {
                            emitted_text = true;
                            let _ = tx.send(Ok(LlmEvent::TextDelta(content.to_owned())));
                        }
                    }
                }
            }

            if is_eof {
                break;
            }
        }

        if !emitted_text && pending_tools.is_empty() {
            record_wire_detail(
                &err_slot,
                format!(
                    "Codex SSE stream closed without response.completed or output on account '{}' (model '{}', events_seen={:?})",
                    account_key, model, seen_events
                ),
            );
        }
        if flush_pending_tools(pending_tools, &tx, &account_key, &err_slot) {
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
        new_error_detail_slot(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drain_next_sse_line_reassembles_multibyte_utf8_across_chunks() {
        let full_line = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Mesut Özil — Đội tuyển Đức\"}\n";
        let bytes = full_line.as_bytes();
        // Split right inside the 2-byte UTF-8 sequence for 'Ö' (0xC3 0x96) and 3-byte 'Đ' / 'ệ'
        let mut raw_buf = Vec::new();
        for b in bytes {
            raw_buf.push(*b);
            if *b != b'\n' {
                assert!(drain_next_sse_line(&mut raw_buf).is_none());
            }
        }
        let line = drain_next_sse_line(&mut raw_buf).expect("complete SSE line");
        assert!(raw_buf.is_empty());
        assert_eq!(
            line,
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Mesut Özil — Đội tuyển Đức\"}"
        );
    }

    #[test]
    fn extract_message_item_text_reads_done_item_content() {
        let item = json!({
            "type": "message",
            "role": "assistant",
            "content": [
                { "type": "output_text", "text": "Mesut Özil sinh ngày 15/10/1988." }
            ]
        });
        assert_eq!(
            extract_message_item_text(&item),
            "Mesut Özil sinh ngày 15/10/1988."
        );
    }
}
