use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

use super::harness::*;

#[test]
fn manual_final_commits_history_then_enqueues_one_stt_without_partial() {
    let (control, mut messages) = mpsc::channel(2);
    let (audio, _) = mpsc::channel(1);
    let providers = Arc::new(ProviderSet::with_vad(
        Arc::new(FakeVad),
        Arc::new(FakeAsr {
            final_text: "BẠN LÀM ĐƯỢC GÌ".into(),
        }),
    ));
    assert_eq!(providers.vad_adapter(), "fake_vad");
    let mut actor = SessionActor::new("session".into(), control, audio, 2, providers).unwrap();
    let packet = uplink_packet();

    actor.on_client_message(ClientMessage::listen(ListenCommand::Start {
        mode: ListenMode::Manual,
    }));
    assert!(actor.on_binary(packet.as_bytes().to_vec()));
    actor.on_client_message(ClientMessage::listen(ListenCommand::Stop));
    wait_for_worker(&mut actor);

    assert_eq!(actor.phase(), SessionPhase::Ready);
    assert_eq!(actor.dialogue_history(), &["Bạn làm được gì"]);
    let message = messages.try_recv().unwrap();
    let payload: serde_json::Value = serde_json::from_str(message.as_text().unwrap()).unwrap();
    assert_eq!(payload["session_id"], "session");
    assert_eq!(payload["type"], "stt");
    assert_eq!(payload["text"], "Bạn làm được gì");
    assert!(messages.try_recv().is_err());
}

#[test]
fn manual_empty_final_does_not_commit_or_enqueue_stt() {
    let (control, mut messages) = mpsc::channel(1);
    let (audio, _) = mpsc::channel(1);
    let providers = Arc::new(ProviderSet::new(Arc::new(FakeAsr {
        final_text: "  ".into(),
    })));
    let mut actor = SessionActor::new("session".into(), control, audio, 2, providers).unwrap();
    let packet = uplink_packet();

    actor.on_client_message(ClientMessage::listen(ListenCommand::Start {
        mode: ListenMode::Manual,
    }));
    assert!(actor.on_binary(packet.as_bytes().to_vec()));
    actor.on_client_message(ClientMessage::listen(ListenCommand::Stop));
    wait_for_worker(&mut actor);

    assert_eq!(actor.phase(), SessionPhase::Ready);
    assert!(actor.dialogue_history().is_empty());
    assert!(messages.try_recv().is_err());
}

#[test]
fn manual_failed_final_does_not_commit_or_enqueue_stt() {
    let (control, mut messages) = mpsc::channel(1);
    let (audio, _) = mpsc::channel(1);
    let providers = Arc::new(ProviderSet::new(Arc::new(FailingAsr)));
    let mut actor = SessionActor::new("session".into(), control, audio, 2, providers).unwrap();
    let packet = uplink_packet();

    actor.on_client_message(ClientMessage::listen(ListenCommand::Start {
        mode: ListenMode::Manual,
    }));
    assert!(actor.on_binary(packet.as_bytes().to_vec()));
    actor.on_client_message(ClientMessage::listen(ListenCommand::Stop));
    wait_for_worker(&mut actor);

    assert_eq!(actor.phase(), SessionPhase::Ready);
    assert!(actor.dialogue_history().is_empty());
    assert!(messages.try_recv().is_err());
}

#[test]
fn dialogue_history_evicts_the_oldest_manual_final_at_its_configured_limit() {
    let (control, _) = mpsc::channel(2);
    let (audio, _) = mpsc::channel(1);
    let providers = Arc::new(ProviderSet::new(Arc::new(FakeAsr {
        final_text: "xin chào".into(),
    })));
    let mut actor =
        SessionActor::new_with_history("session".into(), control, audio, 2, providers, 1).unwrap();
    let packet = uplink_packet();

    for _ in 0..2 {
        actor.on_client_message(ClientMessage::listen(ListenCommand::Start {
            mode: ListenMode::Manual,
        }));
        assert!(actor.on_binary(packet.as_bytes().to_vec()));
        actor.on_client_message(ClientMessage::listen(ListenCommand::Stop));
        wait_for_worker(&mut actor);
    }

    assert_eq!(actor.dialogue_history(), &["xin chào"]);
}

#[test]
fn manual_capture_overflow_stops_feeding_the_asr_stream() {
    let (control, _) = mpsc::channel(1);
    let (audio, _) = mpsc::channel(1);
    let pushed_frames = Arc::new(AtomicUsize::new(0));
    let providers = Arc::new(ProviderSet::new(Arc::new(CountingAsr {
        pushed_frames: Arc::clone(&pushed_frames),
    })));
    let mut actor = SessionActor::new("session".into(), control, audio, 1, providers).unwrap();
    let packet = uplink_packet();

    actor.on_client_message(ClientMessage::listen(ListenCommand::Start {
        mode: ListenMode::Manual,
    }));
    assert!(actor.on_binary(packet.as_bytes().to_vec()));
    assert!(!actor.on_binary(packet.as_bytes().to_vec()));
    actor.on_client_message(ClientMessage::listen(ListenCommand::Stop));

    for _ in 0..100 {
        if pushed_frames.load(Ordering::Relaxed) == 1 {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }

    assert_eq!(pushed_frames.load(Ordering::Relaxed), 1);
    assert!(actor.dialogue_history().is_empty());
}
