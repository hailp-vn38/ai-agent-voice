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
pub struct ProviderRuntimeAdmission(Arc<Mutex<State>>);
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
    pub fn new(total: usize, reserved_voice: usize) -> Self {
        assert!(total > 0 && reserved_voice > 0 && reserved_voice <= total);
        Self(Arc::new(Mutex::new(State {
            total,
            reserved_voice,
            voice: 0,
            diagnostic: 0,
        })))
    }
    pub fn try_admit(
        &self,
        workload: ProviderWorkloadClass,
    ) -> Result<ProviderCapacityPermit, ProviderAdmissionError> {
        let mut state = self.0.lock().expect("provider admission poisoned");
        let used = state.voice + state.diagnostic;
        let allowed = match workload {
            ProviderWorkloadClass::Voice => used < state.total,
            // A diagnostic can use only capacity *beyond* the Voice reservation.  Checking the
            // total used count (rather than a diagnostic-local counter) keeps that reservation
            // intact even while Voice work is already running.
            ProviderWorkloadClass::Diagnostic => used < state.total - state.reserved_voice,
        };
        if !allowed {
            return Err(ProviderAdmissionError::Capacity);
        }
        match workload {
            ProviderWorkloadClass::Voice => state.voice += 1,
            ProviderWorkloadClass::Diagnostic => state.diagnostic += 1,
        }
        Ok(ProviderCapacityPermit {
            admission: self.clone(),
            workload,
        })
    }
}
impl Drop for ProviderCapacityPermit {
    fn drop(&mut self) {
        let mut state = self
            .admission
            .0
            .lock()
            .expect("provider admission poisoned");
        match self.workload {
            ProviderWorkloadClass::Voice => state.voice -= 1,
            ProviderWorkloadClass::Diagnostic => state.diagnostic -= 1,
        }
    }
}

#[cfg(test)]
mod tests;
