use super::*;
use crate::tools::device_mcp::{
    LlmVisibleTool, McpIncoming, McpOutgoing, parse_tools_page, visible_tools,
};

impl SessionActor {
    pub(super) fn begin_mcp_discovery(&mut self) {
        if !self.mcp.enabled || self.mcp.failed || self.mcp.ready {
            return;
        }
        let Some(id) = self.next_mcp_request_id() else {
            self.mcp.failed = true;
            return;
        };
        self.send_mcp(
            McpOutgoing::Initialize {
                id,
                vision: self.mcp.vision.clone(),
            },
            None,
            PendingMcpKind::Initialize,
            self.mcp.discovery_timeout,
        );
    }

    pub(super) fn on_mcp_message(&mut self, incoming: McpIncoming) {
        let (id, response) = match incoming {
            McpIncoming::Result { id, result } => (id, Ok(result)),
            McpIncoming::Error { id, .. } => (id, Err("device_tool_error")),
            McpIncoming::Notification { .. } => return,
        };
        let Some(pending) = self.mcp.pending.remove(&id) else {
            return;
        };
        if pending
            .generation
            .is_some_and(|generation| generation != self.generation)
        {
            return;
        }
        match pending.kind {
            PendingMcpKind::Initialize => match response {
                Ok(_) => self.request_tools_page(None),
                Err(_) => self.mcp_discovery_failed(),
            },
            PendingMcpKind::ToolsList => match response
                .and_then(|value| parse_tools_page(&value).ok_or("malformed_tools_list"))
            {
                Ok((tools, cursor)) => {
                    self.mcp.discovered.extend(tools);
                    if let Some(cursor) = cursor {
                        self.request_tools_page(Some(cursor));
                    } else {
                        self.mcp.visible = visible_tools(
                            std::mem::take(&mut self.mcp.discovered),
                            &self.mcp.allowed_tools,
                        );
                        self.mcp.ready = true;
                        let tools = self
                            .mcp
                            .visible
                            .iter()
                            .map(|tool| format!("{} -> {}", tool.llm_name, tool.original_name))
                            .collect::<Vec<_>>();
                        tracing::info!(
                            event = "mcp_tools_loaded",
                            tool_count = tools.len(),
                            tools = ?tools,
                            "Device MCP tools loaded into the session registry"
                        );
                    }
                }
                Err(_) => self.mcp_discovery_failed(),
            },
            PendingMcpKind::ToolCall { call } => {
                tracing::info!(
                    event = "mcp_tool_result_received",
                    "Device MCP tool result received"
                );
                self.complete_tool_call(call, response)
            }
        }
    }

    fn request_tools_page(&mut self, cursor: Option<String>) {
        let Some(id) = self.next_mcp_request_id() else {
            self.mcp_discovery_failed();
            return;
        };
        self.send_mcp(
            McpOutgoing::ToolsList { id, cursor },
            None,
            PendingMcpKind::ToolsList,
            self.mcp.discovery_timeout,
        );
    }

    fn mcp_discovery_failed(&mut self) {
        self.mcp.failed = true;
        self.mcp.ready = false;
        self.mcp.visible.clear();
        self.mcp
            .pending
            .retain(|_, pending| matches!(pending.kind, PendingMcpKind::ToolCall { .. }));
    }

    pub(super) fn dispatch_device_mcp_tool(&mut self, call: ToolCall, tool: LlmVisibleTool) {
        if !self.mcp.ready {
            self.complete_tool_call(call, Err("mcp_unavailable"));
            return;
        }
        let Some(arguments) = call.arguments.as_object().cloned() else {
            self.complete_tool_call(call, Err("invalid_arguments"));
            return;
        };
        let Some(id) = self.next_mcp_request_id() else {
            self.complete_tool_call(call, Err("request_id_exhausted"));
            return;
        };
        self.send_mcp(
            McpOutgoing::ToolsCall {
                id,
                name: tool.original_name,
                arguments,
            },
            Some(self.generation),
            PendingMcpKind::ToolCall { call },
            self.mcp.call_timeout,
        );
    }

    fn send_mcp(
        &mut self,
        outgoing: McpOutgoing,
        generation: Option<u64>,
        kind: PendingMcpKind,
        timeout: std::time::Duration,
    ) {
        let id = match &outgoing {
            McpOutgoing::Initialize { id, .. }
            | McpOutgoing::ToolsList { id, .. }
            | McpOutgoing::ToolsCall { id, .. } => *id,
        };
        let payload = serde_json::json!({"type":"mcp", "session_id": self.session_id, "payload": outgoing.payload()});
        let Ok(text) = serde_json::to_string(&payload) else {
            self.mcp_discovery_failed();
            return;
        };
        let sent = if generation.is_some() {
            self.send_turn_control(text).is_ok()
        } else {
            self.send_control(text).is_ok()
        };
        if sent {
            if matches!(&kind, PendingMcpKind::ToolCall { .. }) {
                tracing::info!(event = "mcp_tool_call_sent", "Device MCP tool call sent");
            }
            self.mcp.pending.insert(
                id,
                PendingMcpRequest {
                    generation,
                    deadline: Instant::now() + timeout,
                    kind,
                },
            );
        } else if generation.is_some() {
            self.fail_speech_delivery();
        } else {
            self.mcp_discovery_failed();
        }
    }

    fn next_mcp_request_id(&mut self) -> Option<McpRequestId> {
        let next = self.mcp.next_request_id.checked_add(1)?;
        self.mcp.next_request_id = next;
        Some(McpRequestId(next))
    }

    pub(super) fn expire_mcp_requests(&mut self) {
        let now = Instant::now();
        let expired = self
            .mcp
            .pending
            .iter()
            .filter_map(|(id, request)| (request.deadline <= now).then_some(*id))
            .collect::<Vec<_>>();
        for id in expired {
            let Some(request) = self.mcp.pending.remove(&id) else {
                continue;
            };
            match request.kind {
                PendingMcpKind::Initialize | PendingMcpKind::ToolsList => {
                    self.mcp_discovery_failed()
                }
                PendingMcpKind::ToolCall { call } => self.complete_tool_call(call, Err("timeout")),
            }
        }
    }

    /// Drop semantic ownership of in-flight tool calls after a turn is cancelled.
    /// The device may still execute a side effect, but its late response cannot restart the LLM.
    pub(super) fn cancel_pending_mcp_turn(&mut self) {
        self.mcp
            .pending
            .retain(|_, pending| pending.generation.is_none());
    }
}

pub(super) fn normalize_tool_result(result: serde_json::Value, max_chars: usize) -> String {
    let is_error = result
        .get("isError")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    use unicode_normalization::UnicodeNormalization;
    let content = result
        .get("content")
        .and_then(serde_json::Value::as_array)
        .map(|content| {
            content
                .iter()
                .filter_map(|item| item.get("text").and_then(serde_json::Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    let sanitized: String = content
        .nfc()
        .filter(|character| {
            matches!(character, '\n' | '\t')
                || (!character.is_control()
                    && !matches!(*character, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'))
        })
        .collect();
    let mut chars = sanitized.chars();
    let content: String = chars.by_ref().take(max_chars).collect();
    let truncated = chars.next().is_some();
    serde_json::json!({
        "ok": !is_error,
        "code": is_error.then_some("device_tool_error"),
        "content": content,
        "truncated": truncated,
    })
    .to_string()
}

fn parse_json_value(raw: &str) -> Option<serde_json::Value> {
    let raw = raw.trim_start_matches('\u{feff}').trim();
    let value = serde_json::from_str(raw).ok()?;
    match value {
        serde_json::Value::String(inner) => {
            serde_json::from_str(inner.trim_start_matches('\u{feff}').trim()).ok()
        }
        value => Some(value),
    }
}

fn action_response_from_object(value: &serde_json::Value) -> Option<String> {
    let object = value.as_object()?;
    if object.get("success").and_then(serde_json::Value::as_bool) == Some(false) {
        return None;
    }
    if object.get("action").and_then(serde_json::Value::as_str) != Some("RESPONSE") {
        return None;
    }
    object
        .get("response")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|response| !response.is_empty())
        .map(str::to_owned)
}

fn extract_xiaozhi_action_response(value: &serde_json::Value) -> Option<String> {
    if value.get("success").and_then(serde_json::Value::as_bool) == Some(false) {
        return None;
    }
    action_response_from_object(value).or_else(|| {
        value
            .get("vision_analysis")
            .and_then(action_response_from_object)
    })
}

pub(super) fn parse_xiaozhi_direct_response(result: &serde_json::Value) -> Option<String> {
    if result.get("isError").and_then(serde_json::Value::as_bool) == Some(true) {
        return None;
    }
    result.get("content")?.as_array()?.iter().find_map(|item| {
        if item.get("type").and_then(serde_json::Value::as_str) != Some("text") {
            return None;
        }
        let value = parse_json_value(item.get("text")?.as_str()?)?;
        extract_xiaozhi_action_response(&value)
    })
}

/// Removes the known camera image field from JSON tool text before it can enter LLM history.
///
/// This deliberately follows only the explicit top-level wrapper contract rather than scanning
/// arbitrary client-provided JSON recursively.
pub(super) fn redact_photo_data_from_tool_result(
    mut result: serde_json::Value,
) -> serde_json::Value {
    let Some(content) = result
        .get_mut("content")
        .and_then(serde_json::Value::as_array_mut)
    else {
        return result;
    };
    for item in content {
        let Some(raw) = item
            .get("text")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
        else {
            continue;
        };
        let Some(serde_json::Value::Object(mut object)) = parse_json_value(&raw) else {
            continue;
        };
        if object.remove("photo_data").is_some() {
            item["text"] = serde_json::Value::Object(object).to_string().into();
        }
    }
    result
}

pub(super) fn log_action_envelope_shape(result: &serde_json::Value, tool: &str) {
    const MAX_LOGGED_TEXT_ENVELOPES: usize = 8;
    let is_error = result
        .get("isError")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let content = result.get("content").and_then(serde_json::Value::as_array);
    let content_count = content.map_or(0, Vec::len);
    let text_item_count = content
        .map(|items| {
            items
                .iter()
                .filter(|item| item.get("type").and_then(serde_json::Value::as_str) == Some("text"))
                .count()
        })
        .unwrap_or_default();
    let text_envelopes = content
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    (item.get("type").and_then(serde_json::Value::as_str) == Some("text"))
                        .then(|| {
                            item.get("text")
                                .and_then(serde_json::Value::as_str)
                                .map(|raw| match parse_json_value(raw) {
                                    Some(serde_json::Value::Object(object)) => {
                                        let vision = object.get("vision_analysis");
                                        serde_json::json!({
                                            "bytes": raw.len(),
                                            "json_valid": true,
                                            "root_type": "object",
                                            "root_keys": object.keys().filter(|key| matches!(key.as_str(), "success" | "action" | "response" | "vision_analysis" | "photo_data" | "photo_width" | "photo_height")).collect::<Vec<_>>(),
                                            "has_action": object.contains_key("action"),
                                            "has_response": object.contains_key("response"),
                                            "has_vision_analysis": vision.is_some(),
                                            "has_photo_data": object.contains_key("photo_data"),
                                            "nested_action": vision.and_then(|value| value.get("action")).and_then(serde_json::Value::as_str),
                                            "nested_response_present": vision.and_then(|value| value.get("response")).and_then(serde_json::Value::as_str).is_some_and(|text| !text.trim().is_empty()),
                                        })
                                    }
                                    Some(serde_json::Value::String(_)) => serde_json::json!({"bytes": raw.len(), "json_valid": true, "root_type": "string"}),
                                    Some(serde_json::Value::Array(_)) => serde_json::json!({"bytes": raw.len(), "json_valid": true, "root_type": "array"}),
                                    Some(_) => serde_json::json!({"bytes": raw.len(), "json_valid": true, "root_type": "other"}),
                                    None => serde_json::json!({"bytes": raw.len(), "json_valid": false}),
                                })
                        })
                        .flatten()
                })
                .take(MAX_LOGGED_TEXT_ENVELOPES)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    tracing::info!(
        event = "mcp_tool_result_shape",
        tool,
        is_error,
        content_count,
        text_item_count,
        text_items_truncated = text_item_count > text_envelopes.len(),
        text_envelopes = ?text_envelopes,
        "MCP tool result received for action-envelope classification"
    );
}

#[cfg(test)]
mod tests {
    use super::{
        normalize_tool_result, parse_xiaozhi_direct_response, redact_photo_data_from_tool_result,
    };

    #[test]
    fn normalizer_joins_nfc_sanitizes_and_caps_after_sanitization() {
        let result = serde_json::json!({
            "content": [
                {"text": "e\u{301}\u{0000}\u{202e}x"},
                {"text": "\tcd\n"},
                {"image": "ignored"}
            ]
        });
        let normalized: serde_json::Value =
            serde_json::from_str(&normalize_tool_result(result, 4)).unwrap();
        assert_eq!(normalized["content"], "éx\n\t");
        assert_eq!(normalized["truncated"], true);
    }

    #[test]
    fn normalizer_does_not_mark_exact_cap_as_truncated() {
        let result = serde_json::json!({"content": [{"text": "abcd"}]});
        let normalized: serde_json::Value =
            serde_json::from_str(&normalize_tool_result(result, 4)).unwrap();
        assert_eq!(normalized["content"], "abcd");
        assert_eq!(normalized["truncated"], false);
    }

    #[test]
    fn xiaozhi_response_action_is_classified_before_generic_normalization() {
        let result = serde_json::json!({"isError":false,"content":[{"type":"text","text":"{\"success\":true,\"action\":\"RESPONSE\",\"response\":\"Có một chiếc cốc đỏ.\"}"}]});
        assert_eq!(
            parse_xiaozhi_direct_response(&result).as_deref(),
            Some("Có một chiếc cốc đỏ.")
        );
    }

    #[test]
    fn nested_vision_response_ignores_large_photo_data_before_generic_cap() {
        let photo = "A".repeat(13_000);
        let text = serde_json::json!({
            "success": true,
            "photo_data": format!("data:image/jpeg;base64,{photo}"),
            "photo_width": 320,
            "photo_height": 240,
            "vision_analysis": {
                "success": true,
                "action": "RESPONSE",
                "response": "Trên bàn có một chiếc cốc màu đỏ."
            }
        })
        .to_string();
        assert!(text.chars().count() > 4_096);
        let result = serde_json::json!({
            "isError": false,
            "content": [{"type": "text", "text": text}]
        });

        assert_eq!(
            parse_xiaozhi_direct_response(&result).as_deref(),
            Some("Trên bàn có một chiếc cốc màu đỏ.")
        );
    }

    #[test]
    fn double_encoded_xiaozhi_response_is_classified() {
        let wrapped = serde_json::json!({
            "vision_analysis": {
                "success": true,
                "action": "RESPONSE",
                "response": "Đã nhận diện hình ảnh."
            }
        });
        let result = serde_json::json!({
            "content": [{"type": "text", "text": serde_json::to_string(&wrapped.to_string()).unwrap()}]
        });

        assert_eq!(
            parse_xiaozhi_direct_response(&result).as_deref(),
            Some("Đã nhận diện hình ảnh.")
        );
    }

    #[test]
    fn unsuccessful_outer_wrapper_does_not_speak_nested_vision_response() {
        let result = serde_json::json!({
            "content": [{"type": "nonstandard", "text": serde_json::json!({
                "success": false,
                "vision_analysis": {
                    "success": true,
                    "action": "RESPONSE",
                    "response": "Không được phát câu này."
                }
            }).to_string()}]
        });

        assert!(parse_xiaozhi_direct_response(&result).is_none());
    }

    #[test]
    fn generic_camera_wrapper_redacts_photo_data_before_normalization() {
        let photo = "A".repeat(13_000);
        let result = serde_json::json!({
            "content": [{"type": "text", "text": serde_json::json!({
                "photo_data": format!("data:image/jpeg;base64,{photo}"),
                "vision_analysis": {"success": false, "action": "RESPONSE"}
            }).to_string()}]
        });

        let normalized: serde_json::Value = serde_json::from_str(&normalize_tool_result(
            redact_photo_data_from_tool_result(result),
            4_096,
        ))
        .unwrap();
        assert!(!normalized["content"].as_str().unwrap().contains("AAAA"));
        assert!(
            normalized["content"]
                .as_str()
                .unwrap()
                .contains("vision_analysis")
        );
    }

    #[test]
    fn generic_or_unsuccessful_action_is_not_a_direct_response() {
        for result in [
            serde_json::json!({"content":[{"type":"text","text":"{\"volume\":50}"}]}),
            serde_json::json!({"isError":true,"content":[{"type":"text","text":"{\"action\":\"RESPONSE\",\"response\":\"x\"}"}]}),
            serde_json::json!({"content":[{"type":"text","text":"{\"success\":false,\"action\":\"RESPONSE\",\"response\":\"x\"}"}]}),
            serde_json::json!({"content":[{"type":"text","text":"{\"unexpected\":{\"action\":\"RESPONSE\",\"response\":\"x\"}}"}]}),
        ] {
            assert!(parse_xiaozhi_direct_response(&result).is_none());
        }
    }
}
