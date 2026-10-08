use serde_json::json;
use voice_agent_server::tools::device_mcp::{
    DiscoveredTool, McpIncoming, McpOutgoing, McpRequestId, parse_incoming, sanitize_tool_name,
    visible_tools,
};

#[test]
fn sanitizer_is_stable_and_collision_rejects_the_whole_catalog() {
    assert_eq!(sanitize_tool_name("test.set_value"), "test_set_value");
    let visible = visible_tools(vec![
        DiscoveredTool {
            original_name: "a.b".into(),
            description: String::new(),
            input_schema: json!({}),
        },
        DiscoveredTool {
            original_name: "a_b".into(),
            description: String::new(),
            input_schema: json!({}),
        },
        DiscoveredTool {
            original_name: "test.echo".into(),
            description: String::new(),
            input_schema: json!({}),
        },
    ]);
    assert!(visible.is_none());
}

#[test]
fn discovered_tools_include_formerly_denied_names() {
    let visible = visible_tools(vec![
        DiscoveredTool {
            original_name: "test.echo".into(),
            description: String::new(),
            input_schema: json!({}),
        },
        DiscoveredTool {
            original_name: "self.reboot".into(),
            description: String::new(),
            input_schema: json!({}),
        },
    ]);

    let visible = visible.expect("unique catalog is visible");
    assert_eq!(visible.len(), 2);
    assert!(
        visible
            .iter()
            .any(|tool| tool.original_name == "self.reboot")
    );
}

#[test]
fn duplicate_original_name_rejects_the_whole_catalog() {
    let tool = DiscoveredTool {
        original_name: "test.echo".into(),
        description: String::new(),
        input_schema: json!({}),
    };
    assert!(visible_tools(vec![tool.clone(), tool]).is_none());
}

#[test]
fn json_rpc_parser_rejects_invalid_ids_and_preserves_numeric_correlation() {
    assert_eq!(
        parse_incoming(json!({"jsonrpc":"2.0", "id": 7, "result": {}})),
        Some(McpIncoming::Result {
            id: McpRequestId(7),
            result: json!({})
        })
    );
    assert!(parse_incoming(json!({"jsonrpc":"2.0", "id": "7", "result": {}})).is_none());
}

#[test]
fn tools_call_uses_original_device_name_and_object_arguments() {
    let payload = McpOutgoing::ToolsCall {
        id: McpRequestId(42),
        name: "test.set_value".into(),
        arguments: serde_json::Map::from_iter([(String::from("value"), json!(50))]),
    }
    .payload();
    assert_eq!(payload["id"], 42);
    assert_eq!(payload["params"]["name"], "test.set_value");
    assert_eq!(payload["params"]["arguments"], json!({"value":50}));
}
