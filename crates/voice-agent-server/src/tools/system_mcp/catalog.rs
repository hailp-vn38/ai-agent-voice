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
        tools.push(definition("mcp_list_tools", "Read observed External MCP tools and approval state for an Agent; defaults to the current Agent. This does not execute or approve tools.", json!({"agent_key":key,"server_key":key,"page":page}), &[]));
        tools.push(definition(
            "mcp_get_agent_bindings",
            "Read an Agent's External MCP bindings and enabled states.",
            json!({"agent_key":key}),
            &["agent_key"],
        ));
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
