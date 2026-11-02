use super::harness::{
    FakeMuseVmTransport, FakeStreamTurn, FakeVmAccount, SINGLE_COOKIE, build_gateway_muse_provider,
};
use crate::providers::llm::{
    AgentOutputKind, AgentTurnAction, AgentTurnRequest, LlmError, LlmProvider, OperationJournal,
};
use orchestrator_agent_rs::{OperationUpdate, ToolOperation, ToolOperationStatus};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Default)]
struct Journal(Mutex<Vec<OperationUpdate>>);
#[async_trait::async_trait]
impl OperationJournal for Journal {
    async fn record(&self, update: OperationUpdate) -> Result<(), LlmError> {
        self.0.lock().unwrap().push(update);
        Ok(())
    }
}
fn request(key: &str) -> AgentTurnRequest {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    AgentTurnRequest {
        session_key: key.into(),
        action: AgentTurnAction::Send {
            prompt: "Research Rust".into(),
            output: AgentOutputKind::Message,
            filename: None,
        },
        wait_slice: Duration::from_secs(60),
        operation: ToolOperation {
            id: "agent-op".into(),
            invocation_id: "agent-inv".into(),
            provider_ref: None,
            account_key: None,
            remote_session_id: None,
            continuation_key: Some(key.into()),
            batch_index: 0,
            repairs_used: 0,
            batch_paths_json: None,
            user_turns_before: None,
            request_json: Some("{}".into()),
            status: ToolOperationStatus::Submitting,
            artifact_ids_json: None,
            result_json: None,
            remote_deleted_at: None,
            last_activity_at: now,
            deadline_at: now + 3600,
            created_at: now,
        },
    }
}

fn replay(req: &mut AgentTurnRequest, journal: &Journal) {
    for update in journal.0.lock().unwrap().iter() {
        if let Some(status) = &update.status {
            req.operation.status = *status;
        }
        if let Some(value) = &update.account_key {
            req.operation.account_key = Some(value.clone());
        }
        if let Some(value) = &update.remote_session_id {
            req.operation.remote_session_id = Some(value.clone());
        }
        if let Some(value) = update.user_turns_before {
            req.operation.user_turns_before = Some(value);
        }
        if let Some(value) = &update.request_json {
            req.operation.request_json = Some(value.clone());
        }
        if let Some(value) = update.batch_index {
            req.operation.batch_index = value;
        }
    }
}

#[derive(Default)]
struct FailReminderAckJournal {
    saved: Journal,
    calls: std::sync::atomic::AtomicUsize,
}
#[async_trait::async_trait]
impl OperationJournal for FailReminderAckJournal {
    async fn record(&self, update: OperationUpdate) -> Result<(), LlmError> {
        if self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 4 {
            return Err(LlmError::ToolOutcomeUnknown);
        }
        self.saved.record(update).await
    }
}

struct UnwritableJournal;
#[async_trait::async_trait]
impl OperationJournal for UnwritableJournal {
    async fn record(&self, _: OperationUpdate) -> Result<(), LlmError> {
        Err(LlmError::ToolOutcomeUnknown)
    }
}

#[path = "agent_task/connection.rs"]
mod connection;
#[path = "agent_task/lifecycle.rs"]
mod lifecycle;
#[path = "agent_task/outputs.rs"]
mod outputs;
#[path = "agent_task/recovery.rs"]
mod recovery;
