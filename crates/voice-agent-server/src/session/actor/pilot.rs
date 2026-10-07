use super::*;
use crate::session::pilot::PilotAdmission;

impl SessionActor {
    pub fn with_pilot_admission(mut self, admission: PilotAdmission, status: bool) -> Self {
        self.pilot_admission = admission;
        self.pipeline_status = status;
        self
    }

    pub(crate) fn with_pipeline_writer_terminal(mut self, terminal: watch::Receiver<bool>) -> Self {
        self.pipeline_writer_terminal = Some(terminal);
        self
    }

    pub(crate) fn pipeline_request_epoch(&self) -> std::sync::Arc<std::sync::atomic::AtomicU64> {
        self.pipeline_request.clone()
    }

    pub(super) fn admit_pipeline(&mut self) -> bool {
        if !self.pilot_admission.enabled() {
            return true;
        }
        if self.pipeline_permit.is_some() {
            return true;
        }
        if let Some(permit) = self.pilot_admission.try_voice() {
            self.pipeline_permit = Some(permit);
            return true;
        }
        if self.pipeline_status {
            // Synchronous actor admission has no delayed decision; the queued status is fenced
            // by this request's generation before writer delivery.
            let text = r#"{"type":"pipeline","state":"busy","reason":"capacity"}"#.to_owned();
            let request = self
                .pipeline_request
                .load(std::sync::atomic::Ordering::Acquire);
            if self
                .control_tx
                .try_send(OutboundMessage::PipelineStatus { request, text })
                .is_err()
            {
                self.fail_closed();
            }
        } else {
            self.phase = SessionPhase::Closed;
            let _ = self.urgent_tx.try_send(OutboundMessage::Close(1013));
        }
        false
    }

    pub(super) fn release_pipeline_if_idle(&mut self) {
        if self.phase == SessionPhase::Ready
            && self.turn.is_none()
            && self.pipeline_writer_pending.is_empty()
            && !self.asr_runtime.pilot_work_pending()
            && !self.vad_runtime.pilot_work_pending()
            && !self.tts_runtime.pilot_work_pending()
            && !self.llm_runtime.pilot_work_pending()
        {
            self.pipeline_permit = None;
        }
    }

    pub(super) fn retain_pipeline_until_cleanup(&mut self) {
        let Some(permit) = self.pipeline_permit.take() else {
            return;
        };
        let asr = self.asr_runtime.clone();
        let vad = self.vad_runtime.clone();
        let tts = self.tts_runtime.clone();
        let llm = self.llm_runtime.clone();
        let writer = self.pipeline_writer_terminal.take();
        if !asr.pilot_work_pending()
            && !vad.pilot_work_pending()
            && !tts.pilot_work_pending()
            && !llm.pilot_work_pending()
            && writer
                .as_ref()
                .is_none_or(|done| *done.borrow() || done.has_changed().is_err())
        {
            return;
        }
        // The application supervisor continues native acknowledgement routing after WS teardown.
        // Quarantined native work deliberately retains the envelope for process lifetime.
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let _permit = permit;
                while asr.pilot_work_pending()
                    || vad.pilot_work_pending()
                    || tts.pilot_work_pending()
                    || llm.pilot_work_pending()
                    || writer
                        .as_ref()
                        .is_some_and(|done| !*done.borrow() && done.has_changed().is_ok())
                {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            });
        } else {
            // Actor-only synchronous callers cannot supervise cleanup: fail closed.
            std::mem::forget(permit);
        }
    }
}
