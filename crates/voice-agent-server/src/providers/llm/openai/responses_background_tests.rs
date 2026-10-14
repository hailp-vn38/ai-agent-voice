use super::*;
    use crate::providers::llm::{BackgroundMode, BackgroundTurn, ToolDefinition, ToolCall};
    #[test]
    fn strict_function_call_uses_local_call_id_history() {
        let request = LlmRequest {
            messages: vec![
                ChatMessage::User {content:"Find evidence".into()},
                ChatMessage::AssistantToolCalls {calls: vec![ToolCall {
                    id:"call_a".into(),name:"fetch_url".into(),arguments:json!({"url":"https://example.com"})
                }]},
                ChatMessage::ToolResult{tool_call_id:"call_a".into(),content:"ok".into()},
            ],
            tools:vec![ToolDefinition{name:"submit_result".into(),
                description:"Finish".into(), parameters:json!({"type":"object","properties":{},"required":[],"additionalProperties":false})}],
        };
        let body=turn_body("fixture",&request,&BackgroundMode::Tools).unwrap();
        assert_eq!(body["store"],false);
        assert_eq!(body["input"][1]["type"],"function_call");
        assert_eq!(body["input"][1]["call_id"],"call_a");
        assert_eq!(body["input"][2]["type"],"function_call_output");
        assert_eq!(body["input"][2]["call_id"],"call_a");
        assert_eq!(body["tools"][0]["strict"],true);
        assert_eq!(body["parallel_tool_calls"],false);
        let output=json!({"status":"completed","output":[
            {"type":"reasoning"},
            {"type":"function_call","call_id":"call_b","name":"submit_result","arguments":"{}"}
        ]});
        assert!(matches!(parse_turn(&output,&BackgroundMode::Tools).unwrap().turn,
            BackgroundTurn::ToolCalls(calls) if calls[0].id=="call_b"));
    }
    #[test]
    fn strict_root_format_rejects_tools() {
        let request=LlmRequest::text_turn("Root".into());
        let body=turn_body("fixture",&request,&BackgroundMode::Structured {
            name:"root_decision".into(), schema:json!({"type":"object"})
        }).unwrap();
        assert_eq!(body["text"]["format"]["strict"],true);
        assert!(body.get("tools").is_none());
    }
