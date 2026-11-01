use crate::providers::llm::{AgentOutputKind, AgentTurnAction};

pub(super) fn contract(key: &str, action: &AgentTurnAction) -> Option<String> {
    let (output, filename) = match action {
        AgentTurnAction::Send {
            output, filename, ..
        }
        | AgentTurnAction::Collect { output, filename } => (output, filename),
    };
    let name = filename
        .as_deref()
        .or_else(|| (*output == AgentOutputKind::Markdown).then_some("report.md"))?;
    let name = name
        .rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or("report.md");
    Some(format!(
        "tasks/{}/{name}",
        key.trim_start_matches("muse_task:")
    ))
}
pub(super) fn build(key: &str, action: &AgentTurnAction, text: &str) -> String {
    let directory = format!("tasks/{}/", key.trim_start_matches("muse_task:"));
    let file = contract(key, action)
        .map(|path| {
            format!(" Write the requested output to {path} and attach it using a sandbox:// link.")
        })
        .unwrap_or_default();
    format!(
        "{text}\n\nWork only in {directory}.{file} Finish your final answer with [MUSE_TASK_DONE]."
    )
}
