use crate::providers::llm::AgentOutputKind;
use orchestrator_agent_rs::ToolError;
use serde::Deserialize;

#[derive(Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Action {
    #[default]
    Send,
    Collect,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Args {
    #[serde(default)]
    pub(super) action: Action,
    pub(super) prompt: Option<String>,
    #[serde(default)]
    pub(super) output: AgentOutputKind,
    pub(super) filename: Option<String>,
    #[serde(default = "default_wait")]
    pub(super) max_wait_secs: u64,
    #[serde(default)]
    pub(super) delete_session_on_finish: bool,
}
fn default_wait() -> u64 {
    3600
}
pub(super) fn error(code: &str, message: impl Into<String>) -> ToolError {
    ToolError {
        code: code.into(),
        message: message.into(),
    }
}
pub(super) fn unknown(_: ToolError) -> ToolError {
    error(
        "TOOL_OUTCOME_UNKNOWN",
        "Muse operation journal could not be accessed; reconcile before sending again",
    )
}
