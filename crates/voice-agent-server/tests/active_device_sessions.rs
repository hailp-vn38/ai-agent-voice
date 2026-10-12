use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use voice_agent_server::{
    protocol::ClientFeatures,
    services::active_device_sessions::ActiveDeviceSessions,
    session::{DirectSpeechError, DirectSpeechRequest, SessionEvent, SpeechOrigin},
};

fn speech() -> DirectSpeechRequest {
    DirectSpeechRequest {
        request_id: "admin-1".into(),
        text: "hello".into(),
        origin: SpeechOrigin::AdminDirect,
    }
}

fn capabilities() -> ClientFeatures {
    ClientFeatures {
        direct_tts: true,
        ..Default::default()
    }
}

#[tokio::test]
async fn registry_replacement_fences_old_cleanup_and_pending_admission() {
    let registry = Arc::new(ActiveDeviceSessions::default());
    let (old_tx, mut old_rx) = mpsc::channel(1);
    let old_close = CancellationToken::new();
    registry.register(1, "old".into(), capabilities(), old_tx, old_close.clone());
    let caller = registry.clone();
    let pending = tokio::spawn(async move { caller.speak(1, speech()).await });
    let Some(SessionEvent::DirectSpeech { reply, .. }) = old_rx.recv().await else {
        panic!("typed request missing")
    };
    let (new_tx, _new_rx) = mpsc::channel(1);
    registry.register(
        1,
        "new".into(),
        capabilities(),
        new_tx,
        CancellationToken::new(),
    );
    assert!(old_close.is_cancelled());
    registry.unregister(1, "old");
    assert_eq!(registry.current(1).unwrap().session_id, "new");
    reply.send(Ok(())).unwrap();
    assert_eq!(
        pending.await.unwrap(),
        Err(DirectSpeechError::InvalidSession)
    );
    registry.unregister(1, "new");
    assert!(registry.current(1).is_none());
}

#[tokio::test]
async fn registry_capacity_and_timeout_are_not_accepted() {
    let registry = Arc::new(ActiveDeviceSessions::default());
    let (tx, mut rx) = mpsc::channel(1);
    tx.try_send(SessionEvent::ClientAudio(Vec::new())).unwrap();
    registry.register(
        1,
        "one".into(),
        capabilities(),
        tx,
        CancellationToken::new(),
    );
    assert_eq!(
        registry.speak(1, speech()).await,
        Err(DirectSpeechError::CapacityReached)
    );
    let _ = rx.recv().await;
    let caller = registry.clone();
    let pending = tokio::spawn(async move { caller.speak(1, speech()).await });
    let Some(SessionEvent::DirectSpeech { reply, .. }) = rx.recv().await else {
        panic!("typed request missing")
    };
    assert_eq!(
        pending.await.unwrap(),
        Err(DirectSpeechError::CapacityReached)
    );
    assert!(
        reply.is_closed(),
        "actor must discard expired admission before starting speech"
    );
}
