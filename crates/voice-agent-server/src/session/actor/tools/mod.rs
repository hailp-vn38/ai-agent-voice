mod builtin_actions;
mod catalog;
mod external;
mod round;

use super::*;
use catalog::ToolTarget;

pub(super) fn is_builtin_tool_name(name: &str) -> bool {
    catalog::is_builtin_tool_name(name)
}

#[cfg(test)]
use crate::providers::llm::ToolCall;
#[cfg(test)]
use crate::tools::builtin::SWITCH_TEMPLATE_TOOL_NAME;
#[cfg(test)]
use tokio::sync::{mpsc, watch};

#[cfg(test)]
mod tests;
