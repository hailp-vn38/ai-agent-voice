//! Immutable admission catalog: LLM-visible definitions, routing and result-delivery selection.

use super::*;

use crate::{
    config::McpResultDelivery,
    providers::llm::ToolDefinition,
    tools::{
        builtin::{
            BuiltinTool, EXIT_TOOL_NAME, SWITCH_TEMPLATE_TOOL_NAME, exit_tool_definition,
            is_task_suite_tool_name, switch_template_tool_definition,
        },
        device_mcp::LlmVisibleTool,
        external_mcp::{ResolvedExternalMcp, ResolvedExternalTool},
    },
};

/// A builtin tool must not be shadowed by, or shadow, a Device MCP tool. Sanitized MCP names can
/// never contain a dot, so the dotted server action is unambiguous while the exit tool is filtered.
pub(super) fn is_builtin_tool_name(name: &str) -> bool {
    name == EXIT_TOOL_NAME || name == SWITCH_TEMPLATE_TOOL_NAME || is_task_suite_tool_name(name)
}

/// Where one ToolCall goes, decided from the name the model used and nothing else.
///
/// Every arm is a capability this session was actually admitted with, so resolving to one is a
/// statement about this session's immutable catalog rather than about what exists anywhere.
#[derive(Clone)]
pub(in super::super) enum ToolTarget {
    Builtin(BuiltinTool),
    SystemMcp(crate::tools::system_mcp::SessionSystemMcp),
    DeviceMcp(LlmVisibleTool),
    ExternalMcp(ResolvedExternalMcp, ResolvedExternalTool),
}

impl SessionActor {
    /// Everything this session may call, in the order the model will see it.
    ///
    /// The order is the admission order, not a ranking: the catalog is immutable, so what this
    /// returns is a property of the session rather than a snapshot of anything that can change.
    pub(in super::super) fn available_llm_tools(&self) -> Vec<ToolDefinition> {
        let mut tools = vec![exit_tool_definition()];
        if let Some(ref handle) = self.job_submit {
            tools.extend(handle.tool_definitions());
        }
        if !self.switch_catalog.is_empty() {
            tools.push(switch_template_tool_definition(
                &self.switch_catalog.template_keys(),
            ));
        }
        tools.extend(
            self.mcp
                .visible
                .iter()
                .filter(|tool| !is_builtin_tool_name(&tool.llm_name))
                .map(|tool| ToolDefinition {
                    name: tool.llm_name.clone(),
                    description: tool.description.clone(),
                    parameters: tool.input_schema.clone(),
                }),
        );
        tools.extend(self.external_mcp.servers().iter().flat_map(|server| {
            server
                .tools
                .iter()
                .filter(|tool| !is_builtin_tool_name(&tool.llm_name))
                .map(|tool| ToolDefinition {
                    name: tool.llm_name.clone(),
                    description: tool.description.clone(),
                    parameters: tool.input_schema.clone(),
                })
        }));
        if let Some(system) = &self.system_mcp {
            tools.extend(
                system
                    .tools
                    .iter()
                    .filter(|tool| !is_builtin_tool_name(&tool.name))
                    .cloned(),
            );
        }
        tools
    }

    /// Whether this session holds a tool that can change the final answer.
    pub(in super::super) fn offers_answer_changing_tools(&self) -> bool {
        (self.job_submit.is_some() && self.llm_runtime.supports_tools())
            || !self.mcp.visible.is_empty()
            || !self.external_mcp.is_empty()
            || self
                .system_mcp
                .as_ref()
                .is_some_and(|system| !system.tools.is_empty())
    }

    pub(in super::super) fn resolve_tool(&self, name: &str) -> Option<ToolTarget> {
        if is_task_suite_tool_name(name) {
            return self
                .job_submit
                .as_ref()
                .map(|_| ToolTarget::Builtin(BuiltinTool::TaskSuite));
        }
        if name == EXIT_TOOL_NAME {
            return Some(ToolTarget::Builtin(BuiltinTool::EndConversation));
        }
        if name == SWITCH_TEMPLATE_TOOL_NAME {
            return Some(ToolTarget::Builtin(BuiltinTool::SwitchTemplate));
        }
        if let Some(tool) = self
            .mcp
            .visible
            .iter()
            .find(|tool| tool.llm_name == name)
            .cloned()
        {
            return Some(ToolTarget::DeviceMcp(tool));
        }
        if let Some(system) = &self.system_mcp
            && system.tools.iter().any(|tool| tool.name == name)
        {
            return Some(ToolTarget::SystemMcp(system.clone()));
        }
        self.external_mcp
            .find(name)
            .map(|(server, tool)| ToolTarget::ExternalMcp(server.clone(), tool.clone()))
    }

    /// Resolves how this round's results are delivered once all calls terminalize.
    pub(in super::super) fn round_result_delivery(&self, calls: &[ToolCall]) -> McpResultDelivery {
        calls
            .iter()
            .fold(McpResultDelivery::Silent, |selected, call| {
                if is_task_suite_tool_name(&call.name) {
                    return McpResultDelivery::LlmThenTts;
                }
                let delivery = self
                    .mcp
                    .visible
                    .iter()
                    .find(|tool| tool.llm_name == call.name)
                    .and_then(|tool| self.mcp.tool_delivery.get(&tool.original_name))
                    .copied()
                    .unwrap_or(self.mcp.result_delivery);
                if self.external_mcp.find(&call.name).is_some()
                    || self.system_mcp.as_ref().is_some_and(|system| {
                        system.tools.iter().any(|tool| tool.name == call.name)
                    })
                {
                    return McpResultDelivery::LlmThenTts;
                }
                match (selected, delivery) {
                    (McpResultDelivery::LlmThenTts, _) | (_, McpResultDelivery::LlmThenTts) => {
                        McpResultDelivery::LlmThenTts
                    }
                    (McpResultDelivery::DirectTts, _) | (_, McpResultDelivery::DirectTts) => {
                        McpResultDelivery::DirectTts
                    }
                    _ => McpResultDelivery::Silent,
                }
            })
    }
}
