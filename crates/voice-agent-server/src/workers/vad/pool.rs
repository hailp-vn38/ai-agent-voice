//! Native sessions are opened and warmed by the bounded materializer, then leased to streams.
use crate::providers::{VadError, VadInput, VadProbability, VadProvider, VadSession};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

pub(super) struct VadSessionPool(
    Mutex<Vec<Box<dyn VadSession>>>,
    Arc<AtomicBool>,
    super::super::NativeReadiness,
);
impl VadSessionPool {
    pub fn initialize(provider: &dyn VadProvider, count: usize) -> Result<Arc<Self>, VadError> {
        let mut sessions = Vec::with_capacity(count);
        let mut readiness = super::super::NativeReadiness::default();
        for _ in 0..count {
            let started = std::time::Instant::now();
            let mut session = provider.open()?;
            readiness.initialization += started.elapsed();
            let started = std::time::Instant::now();
            let probability = session.push(VadInput {
                pcm: vec![0.0; 512],
                start_sample: 0,
            })?;
            if probability.start_sample != 0
                || probability.end_sample != 512
                || !probability.probability.is_finite()
                || !(0.0..=1.0).contains(&probability.probability)
            {
                return Err(VadError::Failed("invalid VAD readiness response".into()));
            }
            session.reset()?;
            readiness.warmup += started.elapsed();
            sessions.push(session);
        }
        Ok(Arc::new(Self(
            Mutex::new(sessions),
            Arc::new(AtomicBool::new(true)),
            readiness,
        )))
    }
    pub fn readiness(&self) -> super::super::NativeReadiness {
        self.2
    }
    pub fn health_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.1)
    }
    pub fn take(self: &Arc<Self>) -> Option<Box<dyn VadSession>> {
        if !self.1.load(Ordering::Acquire) {
            return None;
        }
        let session = self.0.lock().expect("VAD pool poisoned").pop()?;
        Some(Box::new(RetainedSession {
            session: Some(session),
            pool: Arc::clone(self),
            reusable: true,
            reset: true,
        }))
    }
    pub fn shutdown(&self) {
        let sessions = std::mem::take(&mut *self.0.lock().expect("VAD pool poisoned"));
        drop(sessions);
    }
}
struct RetainedSession {
    session: Option<Box<dyn VadSession>>,
    pool: Arc<VadSessionPool>,
    reusable: bool,
    reset: bool,
}
impl VadSession for RetainedSession {
    fn push(&mut self, input: VadInput) -> Result<VadProbability, VadError> {
        self.reset = false;
        let result = self.session.as_mut().expect("retained session").push(input);
        self.reusable &= result.is_ok();
        if result.is_err() {
            self.pool.1.store(false, Ordering::Release);
        }
        result
    }
    fn reset(&mut self) -> Result<(), VadError> {
        let result = self.session.as_mut().expect("retained session").reset();
        self.reusable &= result.is_ok();
        if result.is_err() {
            self.pool.1.store(false, Ordering::Release);
        }
        self.reset = result.is_ok();
        result
    }
    fn close(&mut self) -> Result<(), VadError> {
        let result = self.session.as_mut().expect("retained session").close();
        self.reusable &= result.is_ok();
        if result.is_err() {
            self.pool.1.store(false, Ordering::Release);
        }
        result?;
        // Closed is published only after reset acknowledged. A reset failure never
        // puts this exact native session back into the available pool.
        self.reset()
    }
}
impl Drop for RetainedSession {
    fn drop(&mut self) {
        let Some(mut session) = self.session.take() else {
            return;
        };
        if !self.reset && session.reset().is_err() {
            self.reusable = false;
        }
        if !self.reusable {
            self.pool.1.store(false, Ordering::Release);
        } else {
            self.pool.0.lock().expect("VAD pool poisoned").push(session);
        }
    }
}
