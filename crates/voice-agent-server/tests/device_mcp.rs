use std::collections::HashSet;

use serde_json::json;
use voice_agent_server::tools::device_mcp::{
    DiscoveredTool, McpIncoming, McpOutgoing, McpRequestId, parse_incoming, sanitize_tool_name,
    visible_tools,
};

#[test]
fn sanitizer_is_stable_and_collision_drops_all_ambiguous_tools() {
    assert_eq!(sanitize_tool_name("test.set_value"), "test_set_value");
    let allowed = HashSet::from(["a.b".to_owned(), "a_b".to_owned(), "test.echo".to_owned()]);
    let visible = visible_tools(
        vec![
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
        ],
        &allowed,
    );
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].llm_name, "test_echo");
    assert_eq!(visible[0].original_name, "test.echo");
}

#[test]
fn discovered_tools_are_visible_without_a_configured_allowlist() {
    let visible = visible_tools(
        vec![
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
        ],
        &HashSet::new(),
    );

    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].original_name, "test.echo");
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
