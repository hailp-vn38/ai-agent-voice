//! Persisted decision identities shared by Root and Child turns.
use super::super::{JobEngine, JobError, model::LlmRunContext};

pub(super) fn run_context(
    job_id: &str,
    agent_id: &str,
    role: &'static str,
    attempt: u32,
) -> LlmRunContext {
    let mut context = LlmRunContext::new(job_id, None, role, "agent_decision", attempt, 0);
    context.agent_run_id = Some(agent_id.to_owned());
    context
}
pub(super) async fn last_turn_id(
    engine: &JobEngine,
    job_id: &str,
    agent_run_id: &str,
) -> Result<String, JobError> {
    engine
        .database
        .last_agent_decision(job_id, agent_run_id)
        .await
}
