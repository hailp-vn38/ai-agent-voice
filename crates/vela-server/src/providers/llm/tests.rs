use futures_util::StreamExt;

use super::*;

#[tokio::test]
async fn tool_stream_intermediate_chunks_do_not_fail_the_round() {
    let chunks = vec![
        Ok(llm::chat::StreamChunk::ToolUseStart {
            index: 0,
            id: "call-1".into(),
            name: "test_set_value".into(),
        }),
        Ok(llm::chat::StreamChunk::ToolUseInputDelta {
            index: 0,
            partial_json: r#"{"value":50}"#.into(),
        }),
        Ok(llm::chat::StreamChunk::ToolUseComplete {
            index: 0,
            tool_call: llm::ToolCall {
                id: "call-1".into(),
                call_type: "function".into(),
                function: llm::FunctionCall {
                    name: "test_set_value".into(),
                    arguments: r#"{"value":50}"#.into(),
                },
            },
        }),
        Ok(llm::chat::StreamChunk::Done {
            stop_reason: "tool_use".into(),
        }),
    ];

    let events = futures_util::stream::iter(chunks)
        .filter_map(|chunk| async move { adapt_stream_chunk(chunk) })
        .collect::<Vec<_>>()
        .await;

    assert_eq!(events.len(), 2);
    assert!(matches!(
        &events[0],
        Ok(LlmEvent::ToolCall(ToolCall { id, name, arguments }))
            if id == "call-1"
                && name == "test_set_value"
                && *arguments == serde_json::json!({"value": 50})
    ));
    assert!(matches!(&events[1], Ok(LlmEvent::Finished)));
}
