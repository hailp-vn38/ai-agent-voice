use super::*;
#[test]
fn strict_chat_tool_roundtrip() {
    let tool = crate::providers::llm::ToolDefinition {
        name: "submit_result".into(),
        description: "Finish".into(),
        parameters: json!({"type":"object","properties":{"summary":{"type":"string"}},"required":["summary"],"additionalProperties":false}),
    };
    let request = LlmRequest {
        messages: vec![
            ChatMessage::User {
                content: "work".into(),
            },
            ChatMessage::AssistantToolCalls {
                calls: vec![ToolCall {
                    id: "call_1".into(),
                    name: "submit_result".into(),
                    arguments: json!({"summary":"ok"}),
                }],
            },
            ChatMessage::ToolResult {
                tool_call_id: "call_1".into(),
                content: "ok".into(),
            },
        ],
        tools: vec![tool],
    };
    let body = request_body("model", &request, &BackgroundMode::Tools).unwrap();
    assert_eq!(body["messages"][2]["role"], "tool");
    assert_eq!(body["messages"][2]["tool_call_id"], "call_1");
    assert_eq!(body["parallel_tool_calls"], false);
    assert_eq!(body["tools"][0]["function"]["strict"], true);
    let answer = json!({"choices":[{"message":{"tool_calls":[{"id":"call_2","type":"function","function":{"name":"submit_result","arguments":"{\"summary\":\"done\"}"}}]}}]});
    let parsed = parse_response(&answer, &BackgroundMode::Tools).unwrap();
    assert!(matches!(parsed.turn,BackgroundTurn::ToolCalls(c) if c[0].id=="call_2"));
}
#[test]
fn structured_request_forbids_tools() {
    let req = LlmRequest::text_turn("root".into());
    let body = request_body(
        "model",
        &req,
        &BackgroundMode::Structured {
            name: "root_decision".into(),
            schema: json!({"type":"object"}),
        },
    )
    .unwrap();
    assert_eq!(body["response_format"]["json_schema"]["strict"], true);
    assert!(body.get("tools").is_none());
}


#[test]
fn v3_child_auto_tools_can_finish_with_text() {
    let request=LlmRequest{messages:vec![ChatMessage::User{content:"work".into()}],
        tools:vec![crate::providers::llm::ToolDefinition{
            name:"submit_result".into(),description:"Optional finish".into(),
            parameters:json!({"type":"object","properties":{},"required":[],"additionalProperties":false})
        }]};
    let body=request_body("fixture",&request,&BackgroundMode::ToolsAuto).unwrap();
    assert_eq!(body["tool_choice"],"auto");
    assert_eq!(body["parallel_tool_calls"],false);
    let output=json!({"choices":[{"finish_reason":"stop",
        "message":{"content":"The task is finished."}}]});
    assert!(matches!(parse_response(&output,&BackgroundMode::ToolsAuto).unwrap().turn,
        BackgroundTurn::Text(text) if text=="The task is finished."));
}
