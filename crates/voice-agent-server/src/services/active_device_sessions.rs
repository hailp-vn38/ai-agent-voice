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
            if !session.features.direct_tts {
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
