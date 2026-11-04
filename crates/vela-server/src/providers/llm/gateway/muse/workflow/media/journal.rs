use super::*;
use orchestrator_agent_rs::{OperationUpdate, ToolOperation, ToolOperationStatus};

/// Persists the exact pending request before every remote send, including repairs.
/// The original request fields remain available for recovery signature matching.
pub(super) struct TurnJournal<'a> {
    io: Option<&'a dyn OperationJournal>,
    request: Value,
    deadline: Instant,
    /// The session id was just generated here and no turn has been sent on it yet.
    fresh_session: bool,
}

impl<'a> TurnJournal<'a> {
    pub fn new(
        io: Option<&'a dyn OperationJournal>,
        op: Option<&ToolOperation>,
        deadline: Instant,
        fresh_session: bool,
    ) -> Self {
        let request = op
            .and_then(|op| op.request_json.as_deref())
            .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({}));
        Self {
            io,
            request,
            deadline,
            fresh_session,
        }
    }

    fn check_deadline(&self) -> Result<(), WorkflowFailure> {
        if Instant::now() >= self.deadline {
            Err(WorkflowFailure::Timeout(
                "media operation reached its shared deadline".into(),
            ))
        } else {
            Ok(())
        }
    }

    pub fn attachment_followup_sent(&self) -> bool {
        self.request
            .get("attachment_followup_sent")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    pub fn reset_attachment_followup(&mut self) {
        self.request["attachment_followup_sent"] = json!(false);
    }

    pub async fn send_attachment_followup(
        &mut self,
        lease: &mut VmOperationLease,
        index: u32,
        repairs: u32,
        items: Vec<Value>,
    ) -> Result<(), WorkflowFailure> {
        self.request["attachment_followup_sent"] = json!(true);
        self.send(lease, index, repairs, items).await
    }

    pub fn pending_items(&self) -> Option<Vec<Value>> {
        self.request
            .get("pending_turn")
            .and_then(Value::as_array)
            .cloned()
    }

    async fn record(&self, update: OperationUpdate) -> Result<(), WorkflowFailure> {
        if let Some(io) = self.io {
            io.record(update).await.map_err(|_| {
                WorkflowFailure::OutcomeUnknown("could not persist media checkpoint".into())
            })?;
        }
        Ok(())
    }

    async fn pending(
        &mut self,
        lease: &mut VmOperationLease,
        index: u32,
        repairs: u32,
        items: Vec<Value>,
        paths: Option<&[String]>,
    ) -> Result<(), WorkflowFailure> {
        self.check_deadline()?;
        if self.io.is_none() {
            return Ok(());
        }
        // A fresh session has no turns by construction, and Muse may not serve its history yet.
        // On a reused session a failed read happens before this turn is sent, so the outcome is
        // known: nothing was submitted and the caller may treat it as an ordinary provider error.
        let before = if std::mem::take(&mut self.fresh_session) {
            0
        } else {
            count_remote_user_turns(lease).await.ok_or_else(|| {
                WorkflowFailure::Upstream(
                    UpstreamFailureKind::Transient,
                    "could not count remote user turns before sending".into(),
                )
            })?
        };
        self.check_deadline()?;
        self.request["pending_turn"] = json!(items);
        self.record(OperationUpdate {
            status: Some(ToolOperationStatus::Submitting),
            account_key: lease.account_key().map(str::to_string),
            remote_session_id: Some(lease.session_id().to_string()),
            batch_index: Some(index),
            repairs_used: Some(repairs),
            user_turns_before: Some(before),
            request_json: Some(self.request.to_string()),
            batch_paths_json: paths.map(|paths| json!(paths).to_string()),
            ..Default::default()
        })
        .await
    }

    pub async fn send(
        &mut self,
        lease: &mut VmOperationLease,
        index: u32,
        repairs: u32,
        items: Vec<Value>,
    ) -> Result<(), WorkflowFailure> {
        self.pending(lease, index, repairs, items.clone(), None)
            .await?;
        self.check_deadline()?;
        if let Err(err) = send_media_stream_request(lease, items).await {
            return Err(match err {
                WorkflowFailure::Timeout(_)
                | WorkflowFailure::Upstream(UpstreamFailureKind::Transient, _)
                    if self.io.is_some() =>
                {
                    WorkflowFailure::OutcomeUnknown(
                        "remote send may have been accepted; reconcile before retrying".into(),
                    )
                }
                other => other,
            });
        }
        self.record(OperationUpdate {
            status: Some(ToolOperationStatus::Running),
            remote_session_id: Some(lease.session_id().to_string()),
            batch_index: Some(index),
            repairs_used: Some(repairs),
            ..Default::default()
        })
        .await
    }

    pub async fn complete_batch(
        &mut self,
        lease: &mut VmOperationLease,
        next_index: usize,
        next_batch: Option<&MediaBatch>,
        paths: &[String],
    ) -> Result<(), WorkflowFailure> {
        self.check_deadline()?;
        self.reset_attachment_followup();
        if let Some(batch) = next_batch {
            self.pending(
                lease,
                next_index as u32,
                0,
                build_turn_items(&[], &batch.prompt),
                Some(paths),
            )
            .await
        } else {
            self.record(OperationUpdate {
                status: Some(ToolOperationStatus::Downloading),
                batch_index: Some(next_index as u32),
                repairs_used: Some(0),
                batch_paths_json: Some(json!(paths).to_string()),
                ..Default::default()
            })
            .await
        }
    }
}
