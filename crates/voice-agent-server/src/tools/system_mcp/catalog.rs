use super::SystemToolGrants;
use crate::providers::llm::ToolDefinition;
use serde_json::{Value, json};

pub(super) fn catalog(grants: SystemToolGrants) -> Vec<ToolDefinition> {
    let key = json!({"type":"string","minLength":1,"maxLength":64,"pattern":"^[a-z][a-z0-9_]*$"});
    let page = json!({"type":"integer","minimum":1,"maximum":10000});
    let mut tools = Vec::new();
    if grants.system_read {
        tools.push(definition(
            "system_get_status",
            "Read readiness, uptime, database, provider runtime and active session counts.",
            json!({}),
            &[],
        ));
        tools.push(definition("system_get_resources", "Read the latest OS-visible CPU, RAM and root filesystem snapshot. Estimated reservations are separate from measured memory.", json!({}), &[]));
        tools.push(definition(
            "system_get_logs",
            "Read recent structured operational logs from the in-memory ring buffer with optional level, subsystem, session_id, or keyword filtering.",
            json!({
                "min_level": {"type": "string", "enum": ["error", "warn", "info", "debug", "trace"]},
                "subsystem": {"type": "string", "maxLength": 128},
                "session_id": {"type": "string", "maxLength": 128},
                "q": {"type": "string", "maxLength": 128},
                "limit": {"type": "integer", "minimum": 1, "maximum": 50}
            }),
            &[],
        ));
    }
    if grants.mcp_read {
        tools.push(definition(
            "mcp_list_servers",
            "List safe External MCP server metadata without endpoints or credentials.",
            json!({"page":page,"page_size":{"type":"integer","minimum":1,"maximum":100}}),
            &[],
        ));
        tools.push(definition(
            "mcp_get_server",
            "Read safe metadata of one External MCP server.",
            json!({"server_key":key}),
            &["server_key"],
        ));
        tools.push(definition("mcp_list_tools", "Read observed External MCP tools, approval state and review_revision for an Agent; defaults to the current Voice Agent. Remote callers must supply agent_key. This does not execute or approve tools.", json!({"agent_key":key,"server_key":key,"page":page}), &[]));
        tools.push(definition(
            "mcp_get_agent_bindings",
            "Read an Agent's External MCP bindings and enabled states.",
            json!({"agent_key":key}),
            &["agent_key"],
        ));
    }
    if grants.mcp_discover {
        tools.push(definition(
            "mcp_discover_tools",
            "Discover a bounded catalog and observe contract drift; never approve tools.",
            json!({"server_key":key}),
            &["server_key"],
        ));
    }
    if grants.mcp_manage_request {
        let revision = json!({"type":"integer","minimum":1});
        let enabled = json!({"type":"boolean"});
        tools.push(definition("mcp_request_server_change", "Propose changing a server's enabled state. Only a separate Admin confirmation can apply it.", json!({"server_key":key,"enabled":enabled,"expected_revision":revision}), &["server_key","enabled","expected_revision"]));
        tools.push(definition("mcp_request_agent_binding_change", "Propose an Agent binding change for separate Admin confirmation.", json!({"agent_key":key,"server_key":key,"enabled":enabled,"expected_agent_revision":revision}), &["agent_key","server_key","enabled","expected_agent_revision"]));
        tools.push(definition("mcp_request_tool_review", "Propose a tool review using review_revision from mcp_list_tools as expected_revision, for separate Admin confirmation; never approve from conversation.", json!({"agent_key":key,"server_key":key,"tool_name":{"type":"string","maxLength":128},"fingerprint":{"type":"string","pattern":"^[a-f0-9]{64}$"},"observed_revision":revision,"allow":enabled,"sensitive":enabled,"expected_revision":revision}), &["agent_key","server_key","tool_name","fingerprint","observed_revision","allow","sensitive","expected_revision"]));
    }
    tools
}

fn definition(
    name: &str,
    description: &str,
    properties: Value,
    required: &[&str],
) -> ToolDefinition {
    ToolDefinition {
        name: name.into(),
        description: description.into(),
        parameters: json!({"type":"object","properties":properties,"required":required,"additionalProperties":false}),
    }
}
