//! Publishing seam of the Device Push Channel: the real MQTT client and the recording test
//! adapter are interchangeable behind [`WakePublisher`].

use async_trait::async_trait;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};

/// Why a publish failed (the broker never acknowledged the message).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WakePublishError(pub String);

impl std::fmt::Display for WakePublishError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Publishes a retained, QoS 1 message to `topic`; `Ok` means the broker acknowledged it.
#[async_trait]
pub trait WakePublisher: Send + Sync {
    async fn publish(&self, topic: &str, payload: Vec<u8>) -> Result<(), WakePublishError>;
}

/// Test adapter: records every publish and can be told to fail.
#[derive(Default)]
pub struct RecordingPublisher {
    published: Mutex<Vec<(String, Vec<u8>)>>,
    failing: AtomicBool,
}

impl RecordingPublisher {
    pub fn published(&self) -> Vec<(String, Vec<u8>)> {
        self.published.lock().unwrap().clone()
    }

    pub fn set_failing(&self, failing: bool) {
        self.failing.store(failing, Ordering::SeqCst);
    }
}

#[async_trait]
impl WakePublisher for RecordingPublisher {
    async fn publish(&self, topic: &str, payload: Vec<u8>) -> Result<(), WakePublishError> {
        if self.failing.load(Ordering::SeqCst) {
            return Err(WakePublishError("recording publisher set to fail".into()));
        }
        self.published
            .lock()
            .unwrap()
            .push((topic.to_owned(), payload));
        Ok(())
    }
}
