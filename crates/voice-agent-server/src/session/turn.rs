use std::{
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

/// Bounded RAM-only user side of Dialogue History.
#[derive(Debug)]
pub(crate) struct DialogueHistory {
    messages: Vec<String>,
    max_messages: usize,
}

impl DialogueHistory {
    pub(crate) fn new(max_messages: usize) -> Self {
        Self {
            messages: Vec::new(),
            max_messages,
        }
    }
    pub(crate) fn commit_user(&mut self, text: String) {
        self.commit(text);
    }
    pub(crate) fn commit_assistant(&mut self, text: String) {
        self.commit(text);
    }
    fn commit(&mut self, text: String) {
        if self.messages.len() == self.max_messages {
            self.messages.remove(0);
        }
        self.messages.push(text);
    }
    pub(crate) fn messages(&self) -> &[String] {
        &self.messages
    }
}
