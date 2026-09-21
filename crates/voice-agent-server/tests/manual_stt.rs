use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use tokio::sync::mpsc;
use voice_agent_server::{
    audio::{DownlinkOpusEncoder, DownlinkPcmFrame, Pcm16Mono, PcmF32Mono},
    protocol::{ClientMessage, ListenCommand, ListenMode},
    providers::{AsrError, AsrEvent, AsrProvider, AsrResult, AsrSession, ProviderSet, VadProvider},
    session::{SessionActor, SessionPhase},
};

struct FakeAsr {
    final_text: String,
}

struct FakeVad;

impl VadProvider for FakeVad {
    fn adapter(&self) -> &'static str {
        "fake_vad"
    }
}

impl AsrProvider for FakeAsr {
    fn open(&self) -> Result<Box<dyn AsrSession>, AsrError> {
        Ok(Box::new(FakeSession {
            final_text: self.final_text.clone(),
        }))
    }
}

struct FakeSession {
    final_text: String,
}

struct FailingAsr;

impl AsrProvider for FailingAsr {
    fn open(&self) -> Result<Box<dyn AsrSession>, AsrError> {
        Ok(Box::new(FailingSession))
    }
}

struct FailingSession;

impl AsrSession for FailingSession {
    fn push_pcm(&mut self, _: &PcmF32Mono) -> Result<Vec<AsrEvent>, AsrError> {
        Ok(Vec::new())
    }

    fn finish(&mut self) -> Result<AsrResult, AsrError> {
        Err(AsrError::Failed("deterministic test failure".into()))
    }

    fn cancel(&mut self) {}
}

fn uplink_packet() -> voice_agent_server::audio::OpusPacket {
    DownlinkOpusEncoder::new(65_536)
        .unwrap()
        .encode(DownlinkPcmFrame::try_new(Pcm16Mono::new(vec![0; 1_440])).unwrap())
        .unwrap()
}

impl AsrSession for FakeSession {
    fn push_pcm(&mut self, _: &PcmF32Mono) -> Result<Vec<AsrEvent>, AsrError> {
        Ok(vec![AsrEvent::Partial("xin".into())])
    }

    fn finish(&mut self) -> Result<AsrResult, AsrError> {
        Ok(AsrResult::new(&self.final_text))
    }

    fn cancel(&mut self) {}
}

#[test]
fn manual_final_commits_history_then_enqueues_one_stt_without_partial() {
    let (control, mut messages) = mpsc::channel(2);
    let (audio, _) = mpsc::channel(1);
    let providers = Arc::new(ProviderSet::with_vad(
        Arc::new(FakeVad),
        Arc::new(FakeAsr {
            final_text: "xin chào".into(),
        }),
    ));
    assert_eq!(providers.vad_adapter(), "fake_vad");
    let mut actor = SessionActor::new("session".into(), control, audio, 2, providers).unwrap();
    let packet = uplink_packet();

    actor.on_client_message(ClientMessage::Listen(ListenCommand::Start {
        mode: ListenMode::Manual,
    }));
    assert!(actor.on_binary(packet.as_bytes().to_vec()));
    actor.on_client_message(ClientMessage::Listen(ListenCommand::Stop));

    assert_eq!(actor.phase(), SessionPhase::Ready);
    assert_eq!(actor.dialogue_history(), &["xin chào"]);
    let message = messages.try_recv().unwrap();
    let payload: serde_json::Value = serde_json::from_str(message.as_text().unwrap()).unwrap();
    assert_eq!(payload["session_id"], "session");
    assert_eq!(payload["type"], "stt");
    assert_eq!(payload["text"], "xin chào");
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

    actor.on_client_message(ClientMessage::Listen(ListenCommand::Start {
        mode: ListenMode::Manual,
    }));
    assert!(actor.on_binary(packet.as_bytes().to_vec()));
    actor.on_client_message(ClientMessage::Listen(ListenCommand::Stop));

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

    actor.on_client_message(ClientMessage::Listen(ListenCommand::Start {
        mode: ListenMode::Manual,
    }));
    assert!(actor.on_binary(packet.as_bytes().to_vec()));
    actor.on_client_message(ClientMessage::Listen(ListenCommand::Stop));

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
        actor.on_client_message(ClientMessage::Listen(ListenCommand::Start {
            mode: ListenMode::Manual,
        }));
        assert!(actor.on_binary(packet.as_bytes().to_vec()));
        actor.on_client_message(ClientMessage::Listen(ListenCommand::Stop));
    }

    assert_eq!(actor.dialogue_history(), &["xin chào"]);
}

struct CountingAsr {
    pushed_frames: Arc<AtomicUsize>,
}

impl AsrProvider for CountingAsr {
    fn open(&self) -> Result<Box<dyn AsrSession>, AsrError> {
        Ok(Box::new(CountingSession {
            pushed_frames: Arc::clone(&self.pushed_frames),
        }))
    }
}

struct CountingSession {
    pushed_frames: Arc<AtomicUsize>,
}

impl AsrSession for CountingSession {
    fn push_pcm(&mut self, _: &PcmF32Mono) -> Result<Vec<AsrEvent>, AsrError> {
        self.pushed_frames.fetch_add(1, Ordering::Relaxed);
        Ok(Vec::new())
    }

    fn finish(&mut self) -> Result<AsrResult, AsrError> {
        Ok(AsrResult::new("ignored"))
    }

    fn cancel(&mut self) {}
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

    actor.on_client_message(ClientMessage::Listen(ListenCommand::Start {
        mode: ListenMode::Manual,
    }));
    assert!(actor.on_binary(packet.as_bytes().to_vec()));
    assert!(!actor.on_binary(packet.as_bytes().to_vec()));
    actor.on_client_message(ClientMessage::Listen(ListenCommand::Stop));

    assert_eq!(pushed_frames.load(Ordering::Relaxed), 1);
    assert!(actor.dialogue_history().is_empty());
}
