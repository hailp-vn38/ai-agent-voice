//! Atomic shared-capacity admission for Voice and diagnostic workloads.

use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderWorkloadClass {
    Voice,
    Diagnostic,
    Background,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProviderAdmissionError {
    #[error("provider capacity is exhausted")]
    Capacity,
}

#[derive(Clone)]
pub struct ProviderRuntimeAdmission(Vec<Arc<Mutex<State>>>);
struct State {
    total: usize,
    reserved_voice: usize,
    voice: usize,
    diagnostic: usize,
    idle: Arc<tokio::sync::Notify>,
}

pub struct ProviderCapacityPermit {
    admission: ProviderRuntimeAdmission,
    workload: ProviderWorkloadClass,
}

impl ProviderRuntimeAdmission {
    /// Static eligibility, independent of current contention in composed quotas.
    pub(crate) fn has_background_capacity(&self) -> bool {
        self.0.iter().all(|state| {
            let state = state.lock().expect("provider admission poisoned");
            state.total > state.reserved_voice
        })
    }
    /// Work owned by this view. Composed global counters may include other views.
    pub(crate) fn view_usage(&self) -> usize {
        self.0
            .iter()
            .map(|state| {
                let state = state.lock().expect("provider admission poisoned");
                state.voice + state.diagnostic
            })
            .min()
            .unwrap_or(0)
    }
    pub(crate) fn active_work(&self) -> usize {
        self.0
            .iter()
            .map(|state| {
                let state = state.lock().expect("provider admission poisoned");
                state.voice + state.diagnostic
            })
            .sum()
    }
    pub(crate) async fn wait_until_idle(&self) {
        loop {
            let waiters = {
                let mut waiters = Vec::with_capacity(self.0.len());
                for state in &self.0 {
                    let state = state.lock().expect("provider admission poisoned");
                    let used = state.voice + state.diagnostic;
                    if used == 0 {
                        return;
                    }
                    waiters.push(Box::pin(Arc::clone(&state.idle).notified_owned()));
                }
                waiters
            };
            if waiters.is_empty() {
                return;
            }
            let _ = futures_util::future::select_all(waiters).await;
        }
    }
    pub fn new(total: usize, reserved_voice: usize) -> Self {
        assert!(total > 0 && reserved_voice > 0 && reserved_voice <= total);
        Self(vec![Arc::new(Mutex::new(State {
            total,
            reserved_voice,
            voice: 0,
            diagnostic: 0,
            idle: Arc::new(tokio::sync::Notify::new()),
        }))])
    }
    pub(crate) fn composed(&self, other: &Self) -> Self {
        let mut states = self.0.clone();
        states.extend(other.0.iter().cloned());
        states.sort_by_key(|state| Arc::as_ptr(state) as usize);
        states.dedup_by(|a, b| Arc::ptr_eq(a, b));
        Self(states)
    }
    pub fn try_admit(
        &self,
        workload: ProviderWorkloadClass,
    ) -> Result<ProviderCapacityPermit, ProviderAdmissionError> {
        let mut states: Vec<_> = self
            .0
            .iter()
            .map(|state| state.lock().expect("provider admission poisoned"))
            .collect();
        if states.iter().any(|state| {
            let used = state.voice + state.diagnostic;
            match workload {
                ProviderWorkloadClass::Voice => used >= state.total,
                // A single worker must remain testable while idle; both workloads share its slot.
                ProviderWorkloadClass::Diagnostic if state.total == 1 => used >= state.total,
                ProviderWorkloadClass::Diagnostic => used >= state.total - state.reserved_voice,
                ProviderWorkloadClass::Background => used >= state.total - state.reserved_voice,
            }
        }) {
            return Err(ProviderAdmissionError::Capacity);
        }
        for state in &mut states {
            match workload {
                ProviderWorkloadClass::Voice => state.voice += 1,
                ProviderWorkloadClass::Diagnostic => state.diagnostic += 1,
                ProviderWorkloadClass::Background => state.diagnostic += 1,
            }
        }
        Ok(ProviderCapacityPermit {
            admission: self.clone(),
            workload,
        })
    }
}
impl Drop for ProviderCapacityPermit {
    fn drop(&mut self) {
        for state in &self.admission.0 {
            let mut state = state.lock().expect("provider admission poisoned");
            match self.workload {
                ProviderWorkloadClass::Voice => state.voice -= 1,
                ProviderWorkloadClass::Diagnostic => state.diagnostic -= 1,
                ProviderWorkloadClass::Background => state.diagnostic -= 1,
            }
            if state.voice + state.diagnostic == 0 {
                state.idle.notify_waiters();
            }
        }
    }
}

#[cfg(test)]
mod tests;
