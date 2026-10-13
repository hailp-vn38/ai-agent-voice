use std::sync::atomic::{AtomicU64, Ordering};

/// Shared outbound linearization point for turn-scoped delivery.
///
/// A writer may deliver only generations greater than the most recently
/// invalidated generation.  The actor invalidates before it cancels producers,
/// so queued producer output is never the authority for cancellation.
#[derive(Default)]
pub struct GenerationGate {
    invalidated_through: AtomicU64,
}

impl GenerationGate {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn invalidate(&self, generation: u64) {
        self.invalidated_through
            .fetch_max(generation, Ordering::AcqRel);
    }

    pub fn admits(&self, generation: u64) -> bool {
        generation > self.invalidated_through.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::GenerationGate;

    #[test]
    fn invalidation_rejects_the_old_turn_but_not_a_newer_turn() {
        let gate = GenerationGate::new();
        assert!(gate.admits(1));
        gate.invalidate(1);
        assert!(!gate.admits(1));
        assert!(gate.admits(2));
    }
}
