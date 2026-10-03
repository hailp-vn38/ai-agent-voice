//! Atomic shared-capacity admission for Voice and diagnostic workloads.

use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderWorkloadClass {
    Voice,
    Diagnostic,
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
}

pub struct ProviderCapacityPermit {
    admission: ProviderRuntimeAdmission,
    workload: ProviderWorkloadClass,
}

impl ProviderRuntimeAdmission {
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
    pub fn new(total: usize, reserved_voice: usize) -> Self {
        assert!(total > 0 && reserved_voice > 0 && reserved_voice <= total);
        Self(vec![Arc::new(Mutex::new(State {
            total,
            reserved_voice,
            voice: 0,
            diagnostic: 0,
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
                ProviderWorkloadClass::Diagnostic => used >= state.total - state.reserved_voice,
            }
        }) {
            return Err(ProviderAdmissionError::Capacity);
        }
        for state in &mut states {
            match workload {
                ProviderWorkloadClass::Voice => state.voice += 1,
                ProviderWorkloadClass::Diagnostic => state.diagnostic += 1,
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
            }
        }
    }
}

#[cfg(test)]
mod tests;
