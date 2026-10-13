use super::*;
use crate::services::background::voice::rejection;

pub(in super::super) struct PendingJobSubmit {
    call: ToolCall,
    turn_id: TurnId,
    generation: u64,
    deadline: Instant,
}
impl SessionActor {
    pub(crate) fn job_submit_available_without_tool_support(&self) -> bool {
        self.job_submit.is_some() && !self.llm_runtime.supports_tools()
    }
    pub(crate) fn with_job_mailbox(mut self, mailbox: mpsc::Sender<SessionEvent>) -> Self {
        self.job_mailbox = Some(mailbox.downgrade());
        self
    }
    pub(in super::super) fn execute_job_submit(&mut self, call: ToolCall) {
        if !self.llm_runtime.supports_tools() {
            self.complete_tool_call(call, Ok(rejection("llm_tool_calls_unavailable")));
            return;
        }
        let Some(handle) = self.job_submit.clone() else {
            self.complete_tool_call(call, Ok(rejection("runtime_unavailable")));
            return;
        };
        let Some(mailbox) = self
            .job_mailbox
            .as_ref()
            .and_then(mpsc::WeakSender::upgrade)
        else {
            self.complete_tool_call(call, Ok(rejection("runtime_unavailable")));
            return;
        };
        let Some(turn) = self.turn.as_ref() else {
            self.complete_tool_call(call, Ok(rejection("no_active_turn")));
            return;
        };
        let (turn_id, generation) = (turn.turn_id, turn.generation);
        let budget = self
            .tool_rounds
            .remaining()
            .unwrap_or(Duration::ZERO)
            .min(Duration::from_secs(5));
        if budget.is_zero() {
            self.complete_tool_call(call, Ok(rejection("submission_outcome_unknown")));
            return;
        }
        let key = crate::services::background::model::digest(&format!(
            "{}:{}:{}",
            self.session_id,
            turn_id.get(),
            call.id
        ));
        let session_id = self.session_id.clone();
        let tool_call_id = call.id.clone();
        let arguments = call.arguments.clone();
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            self.complete_tool_call(call, Ok(rejection("runtime_unavailable")));
            return;
        };
        runtime.spawn(async move {
            let outcome=handle.submit(arguments,&key,budget).await;
            let durable_id=outcome["job_id"].as_str().map(str::to_owned);
            let event=SessionEvent::JobSubmitCompleted {session_id,turn_id,generation,tool_call_id,outcome};
            if !matches!(tokio::time::timeout(Duration::from_millis(250),mailbox.send(event)).await,Ok(Ok(()))) {
                tracing::warn!(event="job_submit_receipt_undelivered",job_id=?durable_id,"Durable Job receipt could not enter its Voice mailbox");
            }
        });
        if let Some(batch) = self.tool_batch.as_mut() {
            batch.in_flight = Some(InFlightAsyncCall {
                call: call.clone(),
                turn_id,
                generation,
            });
        }
        self.job_pending = Some(PendingJobSubmit {
            call,
            turn_id,
            generation,
            deadline: Instant::now() + budget + Duration::from_secs(1),
        });
    }
    pub(in super::super) fn complete_job_submit(
        &mut self,
        session_id: String,
        turn_id: TurnId,
        generation: u64,
        call_id: String,
        outcome: serde_json::Value,
    ) {
        if session_id != self.session_id
            || self.generation != generation
            || self.current_turn_id() != Some(turn_id)
            || self.job_pending.as_ref().is_none_or(|pending| {
                pending.turn_id != turn_id
                    || pending.generation != generation
                    || pending.call.id != call_id
            })
            || self
                .tool_batch
                .as_ref()
                .and_then(|batch| batch.in_flight.as_ref())
                .is_none_or(|call| {
                    call.call.id != call_id
                        || call.turn_id != turn_id
                        || call.generation != generation
                })
        {
            return;
        }
        let Some(pending) = self.job_pending.take() else {
            return;
        };
        if let Some(batch) = self.tool_batch.as_mut() {
            batch.in_flight = None;
        }
        self.complete_tool_call(pending.call, Ok(outcome));
    }
    pub(in super::super) fn expire_job_submit(&mut self) {
        if self
            .job_pending
            .as_ref()
            .is_some_and(|pending| Instant::now() >= pending.deadline)
        {
            let Some(pending) = self.job_pending.take() else {
                return;
            };
            if let Some(batch) = self.tool_batch.as_mut() {
                batch.in_flight = None;
            }
            if pending.generation == self.generation
                && self.current_turn_id() == Some(pending.turn_id)
            {
                self.complete_tool_call(pending.call, Ok(rejection("submission_outcome_unknown")));
            }
        }
    }
}
