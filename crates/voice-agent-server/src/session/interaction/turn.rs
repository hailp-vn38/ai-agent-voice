use crate::providers::llm::{ChatMessage, ToolCall};
use std::{
    collections::VecDeque,
    num::NonZeroU64,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

/// Semantic identity of one accepted Active Turn within a Voice Session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TurnId(NonZeroU64);

impl TurnId {
    pub fn new(value: u64) -> Option<Self> {
        NonZeroU64::new(value).map(Self)
    }

    pub fn get(self) -> u64 {
        self.0.get()
    }
}

/// Application-scoped admission control for ASR finalization.
pub struct ActiveTurnLimiter {
    capacity: usize,
    active: AtomicUsize,
}

/// RAII ownership for exactly one admitted Active Turn.
pub(crate) struct ActiveTurnPermit {
    limiter: Arc<ActiveTurnLimiter>,
}

impl ActiveTurnLimiter {
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0);
        Self {
            capacity,
            active: AtomicUsize::new(0),
        }
    }

    pub(crate) fn try_acquire(&self) -> bool {
        self.active
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
                (active < self.capacity).then_some(active + 1)
            })
            .is_ok()
    }

    pub(crate) fn try_acquire_permit(limiter: &Arc<Self>) -> Option<ActiveTurnPermit> {
        limiter.try_acquire().then(|| ActiveTurnPermit {
            limiter: Arc::clone(limiter),
        })
    }

    pub(crate) fn release(&self) {
        self.active.fetch_sub(1, Ordering::AcqRel);
    }
}

impl Drop for ActiveTurnPermit {
    fn drop(&mut self) {
        self.limiter.release();
    }
}

/// A completed tool round preserves the assistant-tool-call boundary required by providers.
#[derive(Clone, Debug)]
pub(crate) struct CompletedToolRound {
    calls: Vec<ToolCall>,
    results: Vec<ChatMessage>,
}

#[derive(Clone, Debug)]
pub(crate) struct DialogueExchange {
    turn_id: TurnId,
    user: ChatMessage,
    completed_rounds: Vec<CompletedToolRound>,
    assistant: Option<String>,
}

/// Bounded RAM-only dialogue history. Eviction always removes a whole exchange.
#[derive(Debug)]
pub(crate) struct DialogueHistory {
    exchanges: VecDeque<DialogueExchange>,
    max_messages: usize,
    // Preserves the existing actor test accessor while the typed history owns prompt semantics.
    legacy_messages: Vec<String>,
}

impl DialogueHistory {
    pub(crate) fn new(max_messages: usize) -> Self {
        Self {
            exchanges: VecDeque::new(),
            max_messages,
            legacy_messages: Vec::new(),
        }
    }
    pub(crate) fn commit_user(&mut self, turn_id: TurnId, text: String) {
        self.exchanges.push_back(DialogueExchange {
            turn_id,
            user: ChatMessage::User {
                content: text.clone(),
            },
            completed_rounds: Vec::new(),
            assistant: None,
        });
        self.legacy_messages.push(text);
        self.evict(turn_id);
    }
    pub(crate) fn commit_assistant(&mut self, turn_id: TurnId, text: String) {
        let Some(exchange) = self
            .exchanges
            .iter_mut()
            .find(|exchange| exchange.turn_id == turn_id)
        else {
            return;
        };
        exchange.assistant = Some(text.clone());
        self.legacy_messages.push(text);
        self.evict(turn_id);
    }
    pub(crate) fn append_completed_round(
        &mut self,
        turn_id: TurnId,
        calls: Vec<ToolCall>,
        results: Vec<ChatMessage>,
    ) -> bool {
        if calls.is_empty()
            || calls.len() != results.len()
            || results
                .iter()
                .any(|result| !matches!(result, ChatMessage::ToolResult { .. }))
        {
            return false;
        }
        let Some(exchange) = self
            .exchanges
            .iter_mut()
            .find(|exchange| exchange.turn_id == turn_id)
        else {
            return false;
        };
        exchange
            .completed_rounds
            .push(CompletedToolRound { calls, results });
        self.evict(turn_id);
        true
    }
    pub(crate) fn messages_for_prompt(&self, turn_id: TurnId) -> Option<Vec<ChatMessage>> {
        let current = self.exchanges.back()?;
        if current.turn_id != turn_id {
            return None;
        }
        let mut messages = Vec::new();
        for exchange in &self.exchanges {
            messages.push(exchange.user.clone());
            for round in &exchange.completed_rounds {
                messages.push(ChatMessage::AssistantToolCalls {
                    calls: round.calls.clone(),
                });
                messages.extend(round.results.clone());
            }
            if let Some(assistant) = &exchange.assistant {
                messages.push(ChatMessage::AssistantText {
                    content: assistant.clone(),
                });
            }
        }
        Some(messages)
    }
    pub(crate) fn messages(&self) -> &[String] {
        &self.legacy_messages
    }
    fn message_count(&self) -> usize {
        self.exchanges
            .iter()
            .map(|exchange| {
                1 + exchange
                    .completed_rounds
                    .iter()
                    .map(|round| 1 + round.results.len())
                    .sum::<usize>()
                    + usize::from(exchange.assistant.is_some())
            })
            .sum()
    }
    fn evict(&mut self, current: TurnId) {
        while self.message_count() > self.max_messages && self.exchanges.len() > 1 {
            if self
                .exchanges
                .front()
                .is_some_and(|exchange| exchange.turn_id == current)
            {
                break;
            }
            let removed = self.exchanges.pop_front().expect("len checked");
            let remove_count = 1 + usize::from(removed.assistant.is_some());
            self.legacy_messages
                .drain(..remove_count.min(self.legacy_messages.len()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn call(id: &str) -> ToolCall {
        ToolCall {
            id: id.into(),
            name: "tool".into(),
            arguments: json!({}),
        }
    }
    fn result(id: &str) -> ChatMessage {
        ChatMessage::ToolResult {
            tool_call_id: id.into(),
            content: "{}".into(),
        }
    }

    #[test]
    fn materializes_tool_rounds_without_dangling_calls() {
        let first = TurnId::new(1).unwrap();
        let mut history = DialogueHistory::new(20);
        history.commit_user(first, "u".into());
        assert!(history.append_completed_round(first, vec![call("a")], vec![result("a")]));
        assert!(history.append_completed_round(first, vec![call("c")], vec![result("c")]));
        assert_eq!(
            history.messages_for_prompt(first).unwrap(),
            vec![
                ChatMessage::User {
                    content: "u".into()
                },
                ChatMessage::AssistantToolCalls {
                    calls: vec![call("a")]
                },
                result("a"),
                ChatMessage::AssistantToolCalls {
                    calls: vec![call("c")]
                },
                result("c"),
            ]
        );
    }

    #[test]
    fn keeps_current_atom_when_it_exceeds_eviction_target() {
        let current = TurnId::new(2).unwrap();
        let mut history = DialogueHistory::new(1);
        history.commit_user(current, "u".into());
        assert!(history.append_completed_round(current, vec![call("a")], vec![result("a")]));
        assert_eq!(history.messages_for_prompt(current).unwrap().len(), 3);
    }
}
