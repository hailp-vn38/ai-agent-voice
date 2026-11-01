mod builtin_actions;
mod catalog;
mod external;
pub(super) mod job;
mod memory;
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

mod system;

#[cfg(test)]
mod job_tests;
#[cfg(test)]
#[path = "memory_tests.rs"]
mod memory_tests;
#[cfg(test)]
mod system_tests;
