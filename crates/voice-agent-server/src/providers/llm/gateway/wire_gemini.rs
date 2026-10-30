use std::collections::HashMap;

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use futures_util::stream::poll_fn;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::{
    database::unix_seconds,
    providers::llm::{
        ChatMessage, LlmError, LlmEvent, LlmEventStream, LlmOutput, LlmRequest, ToolCall,
    },
    services::llm_gateway::{GatewayLease, vendors::gemini::ANTIGRAVITY_USER_AGENT},
};

use super::{
    config::LlmGatewayConfig,
    provider::{WireError, apply_lease_headers, check_wire_status, drain_next_sse_line},
};

const DAILY_CLOUDCODE_BASE: &str = "https://daily-cloudcode-pa.googleapis.com/v1internal";
const PROD_CLOUDCODE_BASE: &str = "https://cloudcode-pa.googleapis.com/v1internal";
const DEFAULT_FALLBACK_PROJECT: &str = "bamboo-precept-lgxtn";

pub(crate) fn resolve_gemini_model_and_thinking(
    raw_model: &str,
    reasoning_effort: Option<&str>,
) -> (String, Option<String>) {
    let trimmed = raw_model.trim();
    for suffix in ["(high)", "(medium)", "(low)", "(minimal)"] {
        if let Some(stem) = trimmed.strip_suffix(suffix) {
            let level = suffix.trim_matches(|c| c == '(' || c == ')').to_owned();
            return (stem.trim().to_ascii_lowercase(), Some(level));
        }
    }
    let lower = trimmed.to_ascii_lowercase();
    let effort = reasoning_effort
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .filter(|e| matches!(e.as_str(), "minimal" | "low" | "medium" | "high"));

    match lower.as_str() {
        "gemini-4-argon" => {
            let lvl = effort.unwrap_or_else(|| "high".to_owned());
            (format!("gemini-4-argon-{lvl}"), Some(lvl))
        }
        "gemini-4-argon-high" | "gemini-4-argon-medium" | "gemini-4-argon-low" => {
            let lvl = lower.rsplit('-').next().unwrap_or("high").to_owned();
            (lower, Some(lvl))
        }
        "gemini-3.8-flash" => {
            let lvl = effort.unwrap_or_else(|| "medium".to_owned());
            (format!("gemini-3.8-flash-{lvl}"), Some(lvl))
        }
        "gemini-3.8-flash-high" | "gemini-3.8-flash-medium" | "gemini-3.8-flash-low" => {
            let lvl = lower.rsplit('-').next().unwrap_or("medium").to_owned();
            (lower, Some(lvl))
        }
        "gemini-3.7-flash" => {
            let lvl = effort.unwrap_or_else(|| "medium".to_owned());
            ("gemini-3.7-flash-tiered".to_owned(), Some(lvl))
        }
        "gemini-3.7-flash-high" | "gemini-3.7-flash-medium" | "gemini-3.7-flash-low" => {
            let lvl = lower.rsplit('-').next().unwrap_or("medium").to_owned();
            ("gemini-3.7-flash-tiered".to_owned(), Some(lvl))
        }
        "gemini-3.6-flash" => {
            let lvl = effort.unwrap_or_else(|| "medium".to_owned());
            ("gemini-3.6-flash-tiered".to_owned(), Some(lvl))
        }
        "gemini-3.6-flash-high" | "gemini-3.6-flash-medium" | "gemini-3.6-flash-low" => {
            let lvl = lower.rsplit('-').next().unwrap_or("medium").to_owned();
            ("gemini-3.6-flash-tiered".to_owned(), Some(lvl))
        }
        "gemini-3.1-pro-high" | "gemini-pro-agent" => ("gemini-pro-agent".to_owned(), effort),
        "gemini-3.1-pro-low" => ("gemini-3.1-pro-low".to_owned(), Some("low".to_owned())),
        "gemini-3-flash" => ("gemini-3-flash-agent".to_owned(), effort),
        _ => (lower, effort),
    }
}

fn build_v1internal_url(endpoint_base: &str, action: &str, sse: bool) -> String {
    let trimmed = endpoint_base.trim_end_matches('/');
    let suffix = if sse {
        format!(":{action}?alt=sse")
    } else {
        format!(":{action}")
    };
    if trimmed.contains(":streamGenerateContent") || trimmed.contains(":generateContent") {
        trimmed.to_owned()
    } else if trimmed.ends_with("/v1internal") {
        format!("{trimmed}{suffix}")
    } else {
        format!("{trimmed}/v1internal{suffix}")
    }
}

fn candidate_endpoint_bases(endpoint_base: &str) -> Vec<String> {
    let trimmed = endpoint_base.trim_end_matches('/');
    if trimmed.starts_with("https://daily-cloudcode-pa.googleapis.com") {
        vec![
            DAILY_CLOUDCODE_BASE.to_owned(),
            PROD_CLOUDCODE_BASE.to_owned(),
        ]
    } else {
        vec![trimmed.to_owned()]
    }
}

fn project_id_for_lease(lease: &GatewayLease) -> String {
    lease
        .account_context_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_FALLBACK_PROJECT)
        .to_owned()
}

fn apply_gemini_headers(
    builder: reqwest::RequestBuilder,
    lease: &GatewayLease,
) -> reqwest::RequestBuilder {
    let has_ua = lease
        .extra_headers
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case("user-agent"));
    let builder = apply_lease_headers(builder, lease);
    if has_ua {
        builder
    } else {
        builder.header("User-Agent", ANTIGRAVITY_USER_AGENT)
    }
}

fn normalize_tool_parameters(params: &Value) -> Value {
    if let Some(obj) = params.as_object() {
        let mut cloned = obj.clone();
        cloned.remove("$schema");
        cloned.remove("additionalProperties");
        if cloned.get("type").and_then(Value::as_str) == Some("object")
            && !cloned.contains_key("properties")
        {
            cloned.insert("properties".to_owned(), json!({}));
        }
        return Value::Object(cloned);
    }
    json!({
        "type": "object",
        "properties": {},
    })
}

fn push_or_merge_parts(contents: &mut Vec<Value>, role: &str, mut parts: Vec<Value>) {
    if parts.is_empty() {
        return;
    }
    if let Some(last) = contents.last_mut()
        && last.get("role").and_then(Value::as_str) == Some(role)
        && let Some(arr) = last.get_mut("parts").and_then(Value::as_array_mut)
    {
        arr.append(&mut parts);
        return;
    }
    contents.push(json!({
        "role": role,
        "parts": parts,
    }));
}

fn build_gemini_envelope(
    lease: &GatewayLease,
    config: &LlmGatewayConfig,
    request: &LlmRequest,
    override_contents: Option<Vec<Value>>,
) -> Value {
    let mut system_parts: Vec<Value> = Vec::new();
    let mut contents: Vec<Value> = Vec::new();
    let mut tool_id_to_name: HashMap<String, String> = HashMap::new();

    for msg in &request.messages {
        match msg {
            ChatMessage::System { content } => {
                if !content.trim().is_empty() {
                    system_parts.push(json!({ "text": content }));
                }
            }
            ChatMessage::User { content } => {
                push_or_merge_parts(&mut contents, "user", vec![json!({ "text": content })]);
            }
            ChatMessage::AssistantText { content } => {
                if !content.is_empty() {
                    push_or_merge_parts(&mut contents, "model", vec![json!({ "text": content })]);
                }
            }
            ChatMessage::AssistantToolCalls { calls } => {
                let mut parts = Vec::new();
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
                    tool_id_to_name.insert(call_id.clone(), name.to_owned());
                    let args = match &c.arguments {
                        Value::Object(_) => c.arguments.clone(),
                        Value::String(raw) => serde_json::from_str::<Value>(raw)
                            .ok()
                            .filter(Value::is_object)
                            .unwrap_or_else(|| json!({})),
                        _ => json!({}),
                    };
                    parts.push(json!({
                        "functionCall": {
                            "id": call_id,
                            "name": name,
                            "args": args,
                        }
                    }));
                }
                push_or_merge_parts(&mut contents, "model", parts);
            }
            ChatMessage::ToolResult {
                tool_call_id,
                content,
            } => {
                let fn_name = tool_id_to_name
                    .get(tool_call_id)
                    .cloned()
                    .unwrap_or_else(|| tool_call_id.clone());
                let response_obj = serde_json::from_str::<Value>(content)
                    .ok()
                    .filter(Value::is_object)
                    .unwrap_or_else(|| json!({ "result": content }));
                push_or_merge_parts(
                    &mut contents,
                    "user",
                    vec![json!({
                        "functionResponse": {
                            "id": tool_call_id,
                            "name": fn_name,
                            "response": response_obj,
                        }
                    })],
                );
            }
        }
    }

    if let Some(custom_contents) = override_contents {
        contents = custom_contents;
    }
    if contents.is_empty() {
        contents.push(json!({
            "role": "user",
            "parts": [{ "text": "..." }],
        }));
    }

    let session_num = (uuid::Uuid::new_v4().as_u128() % 9_000_000_000_000_000_000)
        + 1_000_000_000_000_000_000;
    let mut inner_req = json!({
        "sessionId": format!("-{session_num}"),
        "contents": contents,
    });

    if !system_parts.is_empty() {
        inner_req["systemInstruction"] = json!({
            "role": "user",
            "parts": system_parts,
        });
    }

    let (model, thinking_level) =
        resolve_gemini_model_and_thinking(&config.model, config.reasoning_effort.as_deref());
    let mut generation_config = serde_json::Map::new();
    if let Some(max_tokens) = config.max_tokens {
        generation_config.insert("maxOutputTokens".to_owned(), json!(max_tokens));
    }
    if let Some(pct) = config.temperature_percent {
        generation_config.insert(
            "temperature".to_owned(),
            json!((f32::from(pct) / 100.0).clamp(0.0, 2.0)),
        );
    }
    if let Some(level) = thinking_level {
        generation_config.insert(
            "thinkingConfig".to_owned(),
            json!({
                "includeThoughts": level != "minimal",
                "thinkingLevel": level,
            }),
        );
    }
    if !generation_config.is_empty() {
        inner_req["generationConfig"] = Value::Object(generation_config);
    }

    if !request.tools.is_empty() {
        let declarations: Vec<Value> = request
            .tools
            .iter()
            .filter(|t| !t.name.trim().is_empty())
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "parameters": normalize_tool_parameters(&t.parameters),
                })
            })
            .collect();
        if !declarations.is_empty() {
            inner_req["tools"] = json!([{ "functionDeclarations": declarations }]);
            inner_req["toolConfig"] = json!({
                "functionCallingConfig": {
                    "mode": "VALIDATED"
                }
            });
        }
    }

    let now_ms = unix_seconds().unwrap_or(0).saturating_mul(1000);
    let project = project_id_for_lease(lease);

    json!({
        "project": project,
        "model": model,
        "userAgent": "antigravity",
        "requestId": format!(
            "agent/{}/{now_ms}/{}/1",
            uuid::Uuid::new_v4(),
            uuid::Uuid::new_v4()
        ),
        "request": inner_req,
    })
}

async fn send_gemini_request(
    http: &reqwest::Client,
    endpoint_base: &str,
    action: &str,
    sse: bool,
    lease: &GatewayLease,
    body: &Value,
) -> Result<reqwest::Response, WireError> {
    let bases = candidate_endpoint_bases(endpoint_base);
    let mut last_err = WireError::Other;

    for (idx, base) in bases.iter().enumerate() {
        let url = build_v1internal_url(base, action, sse);
        let mut req_builder = apply_gemini_headers(http.post(&url), lease);
        if sse {
            req_builder = req_builder.header("Accept", "text/event-stream");
        } else {
            req_builder = req_builder.header("Accept", "application/json");
        }
        let resp = match req_builder.json(body).send().await {
            Ok(r) => r,
            Err(_) => {
                last_err = WireError::Other;
                continue;
            }
        };
        let status = resp.status();
        if idx + 1 < bases.len() && matches!(status.as_u16(), 400 | 403 | 404 | 502 | 503) {
            last_err = WireError::HttpRejected;
            continue;
        }
        if status.as_u16() == 403 {
            return Err(WireError::HttpRejected);
        }
        return check_wire_status(resp);
    }

    Err(last_err)
}

fn for_each_visible_part(val: &Value, mut on_part: impl FnMut(&Value)) {
    let response = val.get("response").unwrap_or(val);
    let Some(candidates) = response.get("candidates").and_then(Value::as_array) else {
        return;
    };
    for candidate in candidates {
        let Some(parts) = candidate
            .get("content")
            .and_then(|c| c.get("parts"))
            .and_then(Value::as_array)
        else {
            continue;
        };
        for part in parts {
            if part.get("thought").and_then(Value::as_bool) != Some(true) {
                on_part(part);
            }
        }
    }
}

fn emit_gemini_response_parts(
    val: &Value,
    tool_seq: &mut u64,
    tx: &mpsc::UnboundedSender<Result<LlmEvent, LlmError>>,
) {
    for_each_visible_part(val, |part| {
        if let Some(text) = part.get("text").and_then(Value::as_str)
            && !text.is_empty()
        {
            let _ = tx.send(Ok(LlmEvent::TextDelta(text.to_owned())));
        }
        if let Some(fc) = part.get("functionCall") {
            let name = fc
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim();
            if !name.is_empty() {
                *tool_seq += 1;
                let id = fc
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("call_{name}_{tool_seq}"));
                let arguments = fc.get("args").cloned().unwrap_or_else(|| json!({}));
                let _ = tx.send(Ok(LlmEvent::ToolCall(ToolCall {
                    id,
                    name: name.to_owned(),
                    arguments,
                })));
            }
        }
    });
}

pub(crate) async fn open_sse_stream(
    http: &reqwest::Client,
    endpoint_base: &str,
    lease: &GatewayLease,
    config: &LlmGatewayConfig,
    request: &LlmRequest,
) -> Result<LlmEventStream, WireError> {
    let body = build_gemini_envelope(lease, config, request, None);
    let mut resp = send_gemini_request(
        http,
        endpoint_base,
        "streamGenerateContent",
        true,
        lease,
        &body,
    )
    .await?;

    let (tx, mut rx) = mpsc::unbounded_channel::<Result<LlmEvent, LlmError>>();
    tokio::spawn(async move {
        let mut raw_buf: Vec<u8> = Vec::new();
        let mut tool_seq = 0u64;

        loop {
            let chunk_opt = match resp.chunk().await {
                Ok(Some(c)) => Some(c),
                Ok(None) => None,
                Err(_) => {
                    let _ = tx.send(Err(LlmError::Failed));
                    return;
                }
            };
            let is_eof = chunk_opt.is_none();
            if let Some(chunk) = chunk_opt {
                raw_buf.extend_from_slice(&chunk);
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
                    let _ = tx.send(Ok(LlmEvent::Finished));
                    return;
                }
                if let Ok(val) = serde_json::from_str::<Value>(payload) {
                    emit_gemini_response_parts(&val, &mut tool_seq, &tx);
                }
            }
            if is_eof {
                break;
            }
        }

        let trailing_str = String::from_utf8_lossy(&raw_buf);
        let trailing = trailing_str.trim();
        if let Some(data) = trailing.strip_prefix("data:") {
            let payload = data.trim();
            if payload != "[DONE]"
                && let Ok(val) = serde_json::from_str::<Value>(payload)
            {
                emit_gemini_response_parts(&val, &mut tool_seq, &tx);
            }
        } else if trailing.starts_with('{')
            && let Ok(val) = serde_json::from_str::<Value>(trailing)
        {
            emit_gemini_response_parts(&val, &mut tool_seq, &tx);
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
    let vision_contents = vec![json!({
        "role": "user",
        "parts": [
            { "text": prompt },
            {
                "inlineData": {
                    "mimeType": mime_type,
                    "data": BASE64_STANDARD.encode(image_bytes),
                }
            }
        ]
    })];
    let dummy_req = LlmRequest::text_turn(prompt.to_owned());
    let body = build_gemini_envelope(lease, config, &dummy_req, Some(vision_contents));
    let resp = send_gemini_request(
        http,
        endpoint_base,
        "generateContent",
        false,
        lease,
        &body,
    )
    .await?;
    let val: Value = resp.json().await.map_err(|_| WireError::Other)?;
    let mut collected = String::new();
    for_each_visible_part(&val, |part| {
        if let Some(t) = part.get("text").and_then(Value::as_str) {
            collected.push_str(t);
        }
    });
    let trimmed = collected.trim().to_owned();
    if trimmed.is_empty() {
        return Err(WireError::Other);
    }
    Ok(LlmOutput::Text(trimmed))
}
