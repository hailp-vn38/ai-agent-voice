use super::*;
use crate::providers::llm::{BackgroundMode, BackgroundTurn, ToolCall, ToolDefinition};
#[test]
fn strict_function_call_uses_local_call_id_history() {
    let request = LlmRequest {
        messages: vec![
            ChatMessage::User {
                content: "Find evidence".into(),
            },
            ChatMessage::AssistantToolCalls {
                calls: vec![ToolCall {
                    id: "call_a".into(),
                    name: "fetch_url".into(),
                    arguments: json!({"url":"https://example.com"}),
                }],
            },
            ChatMessage::ToolResult {
                tool_call_id: "call_a".into(),
                content: "ok".into(),
            },
        ],
        tools: vec![ToolDefinition {
            name: "submit_result".into(),
            description: "Finish".into(),
            parameters: json!({"type":"object","properties":{},"required":[],"additionalProperties":false}),
        }],
    };
    let body = turn_body("fixture", &request, &BackgroundMode::Tools).unwrap();
    assert_eq!(body["store"], false);
    assert_eq!(body["input"][1]["type"], "function_call");
    assert_eq!(body["input"][1]["call_id"], "call_a");
    assert_eq!(body["input"][2]["type"], "function_call_output");
    assert_eq!(body["input"][2]["call_id"], "call_a");
    assert_eq!(body["tools"][0]["strict"], true);
    assert_eq!(body["parallel_tool_calls"], false);
    let output = json!({"status":"completed","output":[
        {"type":"reasoning"},
        {"type":"function_call","call_id":"call_b","name":"submit_result","arguments":"{}"}
    ]});
    assert!(
        matches!(parse_turn(&output,&BackgroundMode::Tools).unwrap().turn,
            BackgroundTurn::ToolCalls(calls) if calls[0].id=="call_b")
    );
}
#[test]
fn strict_root_format_rejects_tools() {
    let request = LlmRequest::text_turn("Root".into());
    let body = turn_body(
        "fixture",
        &request,
        &BackgroundMode::Structured {
            name: "root_decision".into(),
            schema: json!({"type":"object"}),
        },
    )
    .unwrap();
    assert_eq!(body["text"]["format"]["strict"], true);
    assert!(body.get("tools").is_none());
}

#[test]
fn v3_child_responses_auto_tools_accept_final_text() {
    let request = LlmRequest {
        messages: vec![ChatMessage::User {
            content: "work".into(),
        }],
        tools: vec![ToolDefinition {
            name: "submit_result".into(),
            description: "Optional finish".into(),
            parameters: json!({"type":"object","properties":{},"required":[],"additionalProperties":false}),
        }],
    };
    let body = turn_body("fixture", &request, &BackgroundMode::ToolsAuto).unwrap();
    assert_eq!(body["tool_choice"], "auto");
    let answer = json!({"status":"completed","output":[{"type":"message","role":"assistant",
        "content":[{"type":"output_text","text":"Done with research"}]}]});
    assert!(
        matches!(parse_turn(&answer,&BackgroundMode::ToolsAuto).unwrap().turn,
        BackgroundTurn::Text(text) if text=="Done with research")
    );
}

#[test]
fn v3_responses_complete_batch_incomplete_and_missing_ids_fail_closed() {
    let batch = json!({"status":"completed","output":[
        {"type":"function_call","call_id":"a","name":"subagent","arguments":"{}"},
        {"type":"function_call","call_id":"b","name":"subagent","arguments":"{}"}
    ]});
    assert!(
        matches!(parse_turn(&batch,&BackgroundMode::ToolsParallel).unwrap().turn,BackgroundTurn::ToolCalls(calls) if calls.len()==2&&calls[0].id=="a"&&calls[1].id=="b")
    );
    let mut missing = batch.clone();
    missing["output"][1]["call_id"] = Value::Null;
    assert!(parse_turn(&missing, &BackgroundMode::ToolsParallel).is_err());
    let mut incomplete = batch;
    incomplete["status"] = json!("incomplete");
    assert!(parse_turn(&incomplete, &BackgroundMode::ToolsParallel).is_err());
    let refusal = json!({"status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"refusal","refusal":"no"}]}]});
    assert!(parse_turn(&refusal, &BackgroundMode::ToolsAuto).is_err());
}

#[test]
fn v3_responses_unqualified_opaque_replay_state_is_rejected_before_tools() {
    for reasoning in [
        json!({"type":"reasoning","encrypted_content":"opaque"}),
        json!({"type":"reasoning","id":"rs_native"}),
    ] {
        let answer = json!({"status":"completed","output":[reasoning,
            {"type":"function_call","call_id":"call","name":"subagent","arguments":"{}"}]});
        assert!(parse_turn(&answer, &BackgroundMode::ToolsParallel).is_err());
    }
}
