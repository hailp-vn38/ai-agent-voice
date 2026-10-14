use crate::{
    protocol::ClientFeatures,
    session::{DirectSpeechError, DirectSpeechRequest, SessionEvent, validate_direct_speech},
};
use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub struct ActiveDeviceSession {
    pub session_id: String,
    pub features: ClientFeatures,
    pub sender: mpsc::Sender<SessionEvent>,
    pub close: CancellationToken,
    last_speech: Option<Instant>,
    snapshots: HashMap<String, ReminderSnapshot>,
}

#[derive(Clone, Debug)]
pub struct ReminderSnapshot {
    pub revision: i64,
    pub item_ids: Vec<String>,
}

#[derive(Default)]
pub struct ActiveDeviceSessions(Mutex<HashMap<i64, ActiveDeviceSession>>);

impl ActiveDeviceSessions {
    pub fn register(
        &self,
        device: i64,
        session_id: String,
        features: ClientFeatures,
        sender: mpsc::Sender<SessionEvent>,
        close: CancellationToken,
    ) {
        let mut entries = self.0.lock().unwrap();
        if let Some(old) = entries.get(&device) {
            old.close.cancel();
        }
        entries.insert(
            device,
            ActiveDeviceSession {
                session_id,
                features,
                sender,
                close,
                last_speech: None,
                snapshots: HashMap::new(),
            },
        );
    }

    pub fn unregister(&self, device: i64, session_id: &str) {
        let mut entries = self.0.lock().unwrap();
        if entries
            .get(&device)
            .is_some_and(|s| s.session_id == session_id)
        {
            entries.remove(&device);
        }
    }

    pub fn current(&self, device: i64) -> Option<ActiveDeviceSession> {
        self.0
            .lock()
            .unwrap()
            .get(&device)
            .filter(|s| !s.close.is_cancelled() && !s.sender.is_closed())
            .cloned()
    }

    /// Best effort after a committed mutation. Reconnect always requests a complete snapshot.
    pub fn sync_required(&self, device: i64, revision: i64) {
        let Some(session) = self.current(device) else {
            return;
        };
        if !session.features.reminders {
            return;
        }
        let text = serde_json::json!({"type":"reminder","event":"sync_required","session_id":session.session_id,"revision":revision}).to_string();
        let _ = session
            .sender
            .try_send(SessionEvent::ReminderControl { text });
    }

    pub fn remember_snapshot(
        &self,
        device: i64,
        session_id: &str,
        snapshot_id: String,
        revision: i64,
        item_ids: Vec<String>,
    ) -> bool {
        let mut entries = self.0.lock().unwrap();
        let Some(session) = entries
            .get_mut(&device)
            .filter(|s| s.session_id == session_id && s.features.reminders)
        else {
            return false;
        };
        if session.snapshots.len() >= 4 {
            session.snapshots.clear();
        }
        session
            .snapshots
            .insert(snapshot_id, ReminderSnapshot { revision, item_ids });
        true
    }

    pub fn acknowledge_snapshot(
        &self,
        device: i64,
        session_id: &str,
        snapshot_id: &str,
        revision: i64,
    ) -> Option<ReminderSnapshot> {
        let mut entries = self.0.lock().unwrap();
        let session = entries
            .get_mut(&device)
            .filter(|s| s.session_id == session_id && s.features.reminders)?;
        let snapshot = session.snapshots.remove(snapshot_id)?;
        (snapshot.revision == revision).then_some(snapshot)
    }

    pub async fn speak(
        &self,
        device: i64,
        request: DirectSpeechRequest,
    ) -> Result<(), DirectSpeechError> {
        validate_direct_speech(&request)?;
        let (reply, received) = oneshot::channel();
        let session_id;
        {
            let mut entries = self.0.lock().unwrap();
            let session = entries
                .get_mut(&device)
                .filter(|s| !s.close.is_cancelled() && !s.sender.is_closed())
                .ok_or(DirectSpeechError::DeviceOffline)?;
            session_id = session.session_id.clone();
            if matches!(
                request.origin,
                crate::session::SpeechOrigin::Reminder { .. }
            ) {
                if !session.features.reminders {
                    return Err(DirectSpeechError::UnsupportedFeature);
                }
            } else if !session.features.direct_tts {
                return Err(DirectSpeechError::UnsupportedFeature);
            }
            if session
                .last_speech
                .is_some_and(|last| last.elapsed() < Duration::from_secs(1))
            {
                return Err(DirectSpeechError::RateLimited);
            }
            session
                .sender
                .try_send(SessionEvent::DirectSpeech { request, reply })
                .map_err(|err| match err {
                    mpsc::error::TrySendError::Full(_) => DirectSpeechError::CapacityReached,
                    mpsc::error::TrySendError::Closed(_) => DirectSpeechError::DeviceOffline,
                })?;
            session.last_speech = Some(Instant::now());
        }
        tokio::time::timeout(Duration::from_secs(2), received)
            .await
            .map_err(|_| DirectSpeechError::CapacityReached)?
            .map_err(|_| DirectSpeechError::DeviceOffline)??;
        if self
            .current(device)
            .is_none_or(|s| s.session_id != session_id)
        {
            return Err(DirectSpeechError::InvalidSession);
        }
        Ok(())
    }
}
