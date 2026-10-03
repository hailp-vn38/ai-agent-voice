//! Retained, warmed native ASR sessions with reset before terminal acknowledgement.
use crate::{
    audio::PcmF32Mono,
    providers::{AsrError, AsrEvent, AsrProvider, AsrResult, AsrSession},
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

pub(super) struct AsrSessionPool(
    Mutex<Vec<Box<dyn AsrSession>>>,
    Arc<AtomicBool>,
    super::NativeReadiness,
);
impl AsrSessionPool {
    pub fn initialize(provider: &dyn AsrProvider, count: usize) -> Result<Arc<Self>, AsrError> {
        let mut sessions = Vec::with_capacity(count);
        let mut readiness = super::NativeReadiness::default();
        for _ in 0..count {
            let started = std::time::Instant::now();
            let mut session = provider.open()?;
            readiness.initialization += started.elapsed();
            let started = std::time::Instant::now();
            session.push_pcm(&PcmF32Mono::new(vec![0.0; 16_000], 16_000))?;
            if session.finish()?.text().len() > 32_768 {
                return Err(AsrError::Failed("invalid ASR readiness response".into()));
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
    pub fn readiness(&self) -> super::NativeReadiness {
        self.2
    }
    pub fn health_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.1)
    }
    pub fn take(self: &Arc<Self>) -> Option<Box<dyn AsrSession>> {
        if !self.1.load(Ordering::Acquire) {
            return None;
        }
        let session = self.0.lock().expect("ASR pool poisoned").pop()?;
        Some(Box::new(RetainedSession {
            session: Some(session),
            pool: Arc::clone(self),
            reusable: true,
            reset: true,
        }))
    }
    pub fn shutdown(&self) {
        drop(std::mem::take(
            &mut *self.0.lock().expect("ASR pool poisoned"),
        ));
    }
}
struct RetainedSession {
    session: Option<Box<dyn AsrSession>>,
    pool: Arc<AsrSessionPool>,
    reusable: bool,
    reset: bool,
}
impl AsrSession for RetainedSession {
    fn push_pcm(&mut self, pcm: &PcmF32Mono) -> Result<Vec<AsrEvent>, AsrError> {
        self.reset = false;
        let result = self
            .session
            .as_mut()
            .expect("retained session")
            .push_pcm(pcm);
        self.reusable &= result.is_ok();
        if result.is_err() {
            self.pool.1.store(false, Ordering::Release);
        }
        result
    }
    fn finish(&mut self) -> Result<AsrResult, AsrError> {
        let result = self.session.as_mut().expect("retained session").finish();
        self.reusable &= result.is_ok();
        if result.is_err() {
            self.pool.1.store(false, Ordering::Release);
        }
        let result = result?;
        self.reset()?;
        Ok(result)
    }
    fn cancel(&mut self) {
        self.reset = false;
        self.session.as_mut().expect("retained session").cancel();
    }
    fn reset(&mut self) -> Result<(), AsrError> {
        let result = self.session.as_mut().expect("retained session").reset();
        self.reusable &= result.is_ok();
        if result.is_err() {
            self.pool.1.store(false, Ordering::Release);
        }
        self.reset = result.is_ok();
        result
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
            self.pool.0.lock().expect("ASR pool poisoned").push(session);
        }
    }
}

impl AsrProvider for Arc<AsrSessionPool> {
    fn open(&self) -> Result<Box<dyn AsrSession>, AsrError> {
        self.take()
            .ok_or_else(|| AsrError::Failed("retained ASR capacity unavailable".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Provider;
    struct Session(usize);
    impl AsrProvider for Provider {
        fn open(&self) -> Result<Box<dyn AsrSession>, AsrError> {
            Ok(Box::new(Session(0)))
        }
    }
    impl AsrSession for Session {
        fn push_pcm(&mut self, _: &PcmF32Mono) -> Result<Vec<AsrEvent>, AsrError> {
            Ok(vec![])
        }
        fn finish(&mut self) -> Result<AsrResult, AsrError> {
            Ok(AsrResult::new(""))
        }
        fn cancel(&mut self) {}
        fn reset(&mut self) -> Result<(), AsrError> {
            self.0 += 1;
            if self.0 > 1 {
                Err(AsrError::Failed("fixture reset failure".into()))
            } else {
                Ok(())
            }
        }
    }
    #[test]
    fn reset_failure_marks_retained_pool_unhealthy_without_replacement() {
        let pool = AsrSessionPool::initialize(&Provider, 1).unwrap();
        let mut session = pool.take().unwrap();
        session
            .push_pcm(&PcmF32Mono::new(vec![0.0; 960], 16_000))
            .unwrap();
        assert!(session.finish().is_err());
        drop(session);
        assert!(!pool.health_flag().load(Ordering::Acquire));
        assert!(pool.take().is_none());
    }
}
