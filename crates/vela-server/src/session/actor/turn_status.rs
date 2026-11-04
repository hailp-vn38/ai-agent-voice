use super::*;

impl SessionActor {
    pub(crate) fn with_turn_status(mut self, enabled: bool) -> Self {
        self.turn_status = enabled;
        self
    }

    pub(super) fn emit_turn_status(&mut self, state: &'static str, reason: Option<&'static str>) {
        if !self.turn_status {
            return;
        }
        let mut payload =
            serde_json::json!({"type":"turn_status", "session_id":self.session_id, "state":state});
        if let Some(turn) = self.current_turn_id() {
            payload["turn_id"] = turn.get().into();
        }
        if let Some(reason) = reason {
            payload["reason"] = reason.into();
        }
        if state == "completed" {
            payload["outcome"] = "normal".into();
        }
        if self.send_control(payload.to_string()).is_err() {
            self.fail_closed();
        }
    }
}
