use serde_json::Value;

pub(super) struct ReceiptOrder {
    session: String,
    turn: Option<u64>,
    completed: bool,
    acknowledgement: String,
}
impl ReceiptOrder {
    pub(super) fn new(session: &str) -> Self {
        Self {
            session: session.into(),
            turn: None,
            completed: false,
            acknowledgement: String::new(),
        }
    }
    // The existing tool-status frame follows recorded ToolResult and precedes LLM continuation.
    pub(super) fn observe(&mut self, frame: &Value) -> Result<bool, &'static str> {
        if frame["type"] == "tool" && frame["name"] == "server_create_job" {
            if frame["session_id"] != self.session {
                return Err("wrong receipt session");
            }
            let turn = frame["turn_id"].as_u64().ok_or("missing receipt turn")?;
            match frame["state"].as_str() {
                Some("started") if self.turn.is_none() => self.turn = Some(turn),
                Some("completed") if self.turn == Some(turn) && !self.completed => {
                    self.completed = true;
                    return Ok(true);
                }
                _ => return Err("failed, duplicate or unmatched Job ToolResult"),
            }
        }
        if (frame["type"] == "llm"
            || (frame["type"] == "tts" && frame["state"] == "sentence_start"))
            && let Some(text) = frame["text"].as_str()
        {
            self.acknowledgement.push_str(text);
            if self.acknowledgement.len() > 65536 {
                return Err("acknowledgement too large");
            }
            if self.acknowledgement.to_ascii_lowercase().contains("queued") && !self.completed {
                return Err("acknowledgement preceded committed ToolResult");
            }
        }
        Ok(false)
    }
    pub(super) fn acknowledged(&self) -> bool {
        self.completed && self.acknowledgement.to_ascii_lowercase().contains("queued")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn tool(state: &str, turn: u64) -> Value {
        json!({"type":"tool","name":"server_create_job","session_id":"session","turn_id":turn,"state":state})
    }
    #[test]
    fn receipt_order_rejects_early_ack_and_wrong_turn() {
        let mut order = ReceiptOrder::new("session");
        assert!(
            order
                .observe(&json!({"type":"llm","text":"Job queued"}))
                .is_err()
        );
        let mut order = ReceiptOrder::new("session");
        order.observe(&tool("started", 1)).unwrap();
        assert!(order.observe(&tool("completed", 2)).is_err());
        assert!(!order.acknowledged());
    }
    #[test]
    fn receipt_order_accepts_matched_result_before_split_ack() {
        let mut order = ReceiptOrder::new("session");
        assert!(!order.observe(&tool("started", 1)).unwrap());
        assert!(order.observe(&tool("completed", 1)).unwrap());
        order
            .observe(&json!({"type":"llm","text":"Job que"}))
            .unwrap();
        order.observe(&json!({"type":"llm","text":"ued"})).unwrap();
        assert!(order.acknowledged());
        assert!(order.observe(&tool("completed", 1)).is_err());
    }
}
