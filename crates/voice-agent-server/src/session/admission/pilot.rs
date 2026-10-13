//! One deployment-wide pilot envelope. Native owners retain guards until acknowledgement.
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct State {
    voice: bool,
    enrollment: bool,
    cold: bool,
}
#[derive(Clone)]
pub struct PilotAdmission {
    enabled: bool,
    state: Arc<Mutex<State>>,
}
#[derive(Clone, Copy)]
enum Work {
    Voice,
    Enrollment,
    Cold,
}
pub struct PilotPermit {
    admission: PilotAdmission,
    work: Work,
}
impl PilotAdmission {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            state: Arc::new(Mutex::new(State::default())),
        }
    }
    pub fn enabled(&self) -> bool {
        self.enabled
    }
    pub fn try_voice(&self) -> Option<PilotPermit> {
        self.try_admit(Work::Voice)
    }
    pub fn try_enrollment(&self) -> Option<PilotPermit> {
        self.try_admit(Work::Enrollment)
    }
    pub fn try_cold(&self) -> Option<PilotPermit> {
        self.try_admit(Work::Cold)
    }
    fn try_admit(&self, work: Work) -> Option<PilotPermit> {
        let mut state = self.state.lock().expect("pilot admission poisoned");
        if self.enabled {
            let busy = state.cold
                || match work {
                    Work::Voice => state.voice,
                    Work::Enrollment => state.enrollment,
                    Work::Cold => state.voice || state.enrollment,
                };
            if busy {
                return None;
            }
            match work {
                Work::Voice => state.voice = true,
                Work::Enrollment => state.enrollment = true,
                Work::Cold => state.cold = true,
            }
        }
        Some(PilotPermit {
            admission: self.clone(),
            work,
        })
    }
}
impl Drop for PilotPermit {
    fn drop(&mut self) {
        if !self.admission.enabled {
            return;
        }
        let mut state = self
            .admission
            .state
            .lock()
            .expect("pilot admission poisoned");
        match self.work {
            Work::Voice => state.voice = false,
            Work::Enrollment => state.enrollment = false,
            Work::Cold => state.cold = false,
        }
    }
}
