//! Wake Signal publishing seam for the Device Push Channel.
//!
//! The scheduler talks only to [`WakePublisher`]; the real MQTT client and the recording
//! test adapter are interchangeable behind it.

use async_trait::async_trait;
use serde::Serialize;
use std::sync::Mutex;

pub const WAKE_SIGNAL_VERSION: u32 = 1;

/// Why a publish failed (the broker never acknowledged the Wake Signal).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WakePublishError(pub String);

impl std::fmt::Display for WakePublishError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Wake Signal payload. Carries no Reminder content, only an identifier and an expiry.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct WakeSignal<'a> {
    pub v: u32,
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub id: &'a str,
    pub expires_at: i64,
}

impl<'a> WakeSignal<'a> {
    pub fn new(reminder_id: &'a str, expires_at_ms: i64) -> Self {
        Self {
            v: WAKE_SIGNAL_VERSION,
            kind: "wake",
            id: reminder_id,
            expires_at: expires_at_ms,
        }
    }

    pub fn to_payload(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("wake signal serializes")
    }
}

/// Publishes a retained, QoS 1 message to `topic`.
#[async_trait]
pub trait WakePublisher: Send + Sync {
    async fn publish(&self, topic: &str, payload: Vec<u8>) -> Result<(), WakePublishError>;
}

/// Test adapter: records every publish and can be told to fail.
#[derive(Default)]
pub struct RecordingPublisher {
    published: Mutex<Vec<(String, Vec<u8>)>>,
    failing: std::sync::atomic::AtomicBool,
}

impl RecordingPublisher {
    pub fn published(&self) -> Vec<(String, Vec<u8>)> {
        self.published.lock().unwrap().clone()
    }

    pub fn set_failing(&self, failing: bool) {
        self.failing
            .store(failing, std::sync::atomic::Ordering::SeqCst);
    }
}

#[async_trait]
impl WakePublisher for RecordingPublisher {
    async fn publish(&self, topic: &str, payload: Vec<u8>) -> Result<(), WakePublishError> {
        if self.failing.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(WakePublishError("recording publisher set to fail".into()));
        }
        self.published
            .lock()
            .unwrap()
            .push((topic.to_owned(), payload));
        Ok(())
    }
}
