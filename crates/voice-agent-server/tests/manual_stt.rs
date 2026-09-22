use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

use tokio::sync::mpsc;
use voice_agent_server::{
    audio::{DownlinkOpusEncoder, DownlinkPcmFrame, Pcm16Mono, PcmF32Mono, VadSegmenterConfig},
    protocol::{ClientMessage, ListenCommand, ListenMode},
    providers::{
        AsrError, AsrEvent, AsrProvider, AsrResult, AsrSession, ProviderSet, VadProvider,
        llm::UnavailableLlm,
    },
    session::{ActiveTurnLimiter, OutboundMessage, SessionActor, SessionPhase, SessionRuntimes},
    workers::{AsrWorkerRuntime, LlmRuntime, VadWorkerRuntime, WorkerRuntimeConfig},
};

struct FakeAsr {
    final_text: String,
}

struct FakeVad;

impl VadProvider for FakeVad {
    fn open(
        &self,
    ) -> Result<
        Box<dyn voice_agent_server::providers::VadSession>,
        voice_agent_server::providers::VadError,
    > {
        Err(voice_agent_server::providers::VadError::Failed(
            "not used by Manual".into(),
        ))
    }

    fn adapter(&self) -> &'static str {
        "fake_vad"
    }
}

fn wait_for_worker(actor: &mut SessionActor) {
    for _ in 0..100 {
        actor.pump_workers();
        if actor.phase() == SessionPhase::Ready {
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!("ASR worker did not reach terminal state");
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

struct SlowAsr;

impl AsrProvider for SlowAsr {
    fn open(&self) -> Result<Box<dyn AsrSession>, AsrError> {
        Ok(Box::new(SlowSession))
    }
}

struct SlowSession;

impl AsrSession for SlowSession {
    fn push_pcm(&mut self, _: &PcmF32Mono) -> Result<Vec<AsrEvent>, AsrError> {
        Ok(Vec::new())
    }

    fn finish(&mut self) -> Result<AsrResult, AsrError> {
        std::thread::sleep(Duration::from_millis(50));
        Ok(AsrResult::new("late"))
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
    wait_for_worker(&mut actor);

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

    actor.on_client_message(ClientMessage::Listen(ListenCommand::Start {
        mode: ListenMode::Manual,
    }));
    assert!(actor.on_binary(packet.as_bytes().to_vec()));
    actor.on_client_message(ClientMessage::Listen(ListenCommand::Stop));
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
        actor.on_client_message(ClientMessage::Listen(ListenCommand::Start {
            mode: ListenMode::Manual,
        }));
        assert!(actor.on_binary(packet.as_bytes().to_vec()));
        actor.on_client_message(ClientMessage::Listen(ListenCommand::Stop));
        wait_for_worker(&mut actor);
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

    for _ in 0..100 {
        if pushed_frames.load(Ordering::Relaxed) == 1 {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }

    assert_eq!(pushed_frames.load(Ordering::Relaxed), 1);
    assert!(actor.dialogue_history().is_empty());
}

struct BoundaryVad;

impl VadProvider for BoundaryVad {
    fn open(
        &self,
    ) -> Result<
        Box<dyn voice_agent_server::providers::VadSession>,
        voice_agent_server::providers::VadError,
    > {
        Ok(Box::new(BoundaryVadSession { frames: 0 }))
    }

    fn adapter(&self) -> &'static str {
        "boundary"
    }
}

struct BoundaryVadSession {
    frames: usize,
}

impl voice_agent_server::providers::VadSession for BoundaryVadSession {
    fn push(
        &mut self,
        input: voice_agent_server::providers::VadInput,
    ) -> Result<
        voice_agent_server::providers::VadProbability,
        voice_agent_server::providers::VadError,
    > {
        self.frames += 1;
        Ok(voice_agent_server::providers::VadProbability {
            start_sample: input.start_sample,
            end_sample: input.start_sample + 512,
            probability: if self.frames == 1 { 1.0 } else { 0.0 },
        })
    }

    fn reset(&mut self) -> Result<(), voice_agent_server::providers::VadError> {
        Ok(())
    }
    fn close(&mut self) -> Result<(), voice_agent_server::providers::VadError> {
        Ok(())
    }
}

#[test]
fn auto_cycle_opens_asr_after_speech_start_and_rearms_only_after_reset_done() {
    let (control, mut messages) = mpsc::channel(2);
    let (audio, _) = mpsc::channel(1);
    let config = WorkerRuntimeConfig {
        max_workers: 1,
        command_capacity: 8,
        final_timeout: Duration::from_secs(1),
        cleanup_grace: Duration::from_secs(1),
    };
    let asr = Arc::new(AsrWorkerRuntime::new(
        Arc::new(FakeAsr {
            final_text: "auto final".into(),
        }),
        config.clone(),
    ));
    let vad = Arc::new(VadWorkerRuntime::new(Arc::new(BoundaryVad), config));
    let mut actor = SessionActor::new_with_runtimes_and_limiter(
        "auto".into(),
        control,
        audio,
        4,
        20,
        SessionRuntimes {
            asr,
            vad,
            llm: Arc::new(LlmRuntime::new(
                Arc::new(UnavailableLlm),
                1,
                Duration::from_secs(60),
            )),
            active_turn_limiter: Arc::new(ActiveTurnLimiter::new(1)),
            vad_segmenter_config: VadSegmenterConfig {
                speech_threshold: 0.5,
                exit_threshold: 0.35,
                min_speech_samples: 512,
                end_silence_samples: 512,
            },
            pre_roll_samples: 0,
        },
    )
    .unwrap();
    let packet = uplink_packet();

    actor.on_client_message(ClientMessage::Listen(ListenCommand::Start {
        mode: ListenMode::Auto,
    }));
    for _ in 0..50 {
        actor.pump_workers();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(actor.phase(), SessionPhase::Listening);
    assert!(actor.on_binary(packet.as_bytes().to_vec()));
    for _ in 0..50 {
        actor.pump_workers();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(actor.on_binary(packet.as_bytes().to_vec()));

    for _ in 0..100 {
        actor.pump_workers();
        if actor.phase() == SessionPhase::Listening && !actor.dialogue_history().is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(actor.phase(), SessionPhase::Listening);
    assert_eq!(actor.dialogue_history(), &["auto final"]);
    assert!(
        matches!(messages.try_recv(), Ok(OutboundMessage::Text(text)) if text.contains("auto final"))
    );
}

#[test]
fn active_turn_capacity_denial_finishes_without_stt_or_history() {
    let runtime = Arc::new(AsrWorkerRuntime::new(
        Arc::new(SlowAsr),
        WorkerRuntimeConfig {
            max_workers: 2,
            command_capacity: 8,
            final_timeout: Duration::from_secs(1),
            cleanup_grace: Duration::from_secs(1),
        },
    ));
    let vad = Arc::new(VadWorkerRuntime::new(
        Arc::new(FakeVad),
        WorkerRuntimeConfig::default(),
    ));
    let limiter = Arc::new(ActiveTurnLimiter::new(1));
    let (first_control, _) = mpsc::channel(1);
    let (first_audio, _) = mpsc::channel(1);
    let (second_control, mut second_messages) = mpsc::channel(1);
    let (second_audio, _) = mpsc::channel(1);
    let mut first = SessionActor::new_with_runtimes_and_limiter(
        "first".into(),
        first_control,
        first_audio,
        2,
        20,
        SessionRuntimes {
            asr: Arc::clone(&runtime),
            vad: Arc::clone(&vad),
            llm: Arc::new(LlmRuntime::new(
                Arc::new(UnavailableLlm),
                1,
                Duration::from_secs(60),
            )),
            active_turn_limiter: Arc::clone(&limiter),
            vad_segmenter_config: VadSegmenterConfig::default(),
            pre_roll_samples: 4_800,
        },
    )
    .unwrap();
    let mut second = SessionActor::new_with_runtimes_and_limiter(
        "second".into(),
        second_control,
        second_audio,
        2,
        20,
        SessionRuntimes {
            asr: runtime,
            vad,
            llm: Arc::new(LlmRuntime::new(
                Arc::new(UnavailableLlm),
                1,
                Duration::from_secs(60),
            )),
            active_turn_limiter: limiter,
            vad_segmenter_config: VadSegmenterConfig::default(),
            pre_roll_samples: 4_800,
        },
    )
    .unwrap();
    let packet = uplink_packet();

    for actor in [&mut first, &mut second] {
        actor.on_client_message(ClientMessage::Listen(ListenCommand::Start {
            mode: ListenMode::Manual,
        }));
        assert!(actor.on_binary(packet.as_bytes().to_vec()));
    }
    first.on_client_message(ClientMessage::Listen(ListenCommand::Stop));
    second.on_client_message(ClientMessage::Listen(ListenCommand::Stop));
    for _ in 0..100 {
        first.pump_workers();
        second.pump_workers();
        if second.phase() == SessionPhase::Ready {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(second.phase(), SessionPhase::Ready);
    assert!(second.dialogue_history().is_empty());
    assert!(second_messages.try_recv().is_err());
}

#[test]
fn replacement_invalidates_a_late_final_without_emitting_stale_stt() {
    let runtime = Arc::new(AsrWorkerRuntime::new(
        Arc::new(SlowAsr),
        WorkerRuntimeConfig {
            max_workers: 2,
            command_capacity: 8,
            final_timeout: Duration::from_secs(1),
            cleanup_grace: Duration::from_secs(1),
        },
    ));
    let vad = Arc::new(VadWorkerRuntime::new(
        Arc::new(FakeVad),
        WorkerRuntimeConfig::default(),
    ));
    let (control, mut messages) = mpsc::channel(2);
    let (audio, _) = mpsc::channel(1);
    let mut actor =
        SessionActor::new_with_runtimes("replacement".into(), control, audio, 2, 20, runtime, vad)
            .unwrap();
    let packet = uplink_packet();

    actor.on_client_message(ClientMessage::Listen(ListenCommand::Start {
        mode: ListenMode::Manual,
    }));
    assert!(actor.on_binary(packet.as_bytes().to_vec()));
    actor.on_client_message(ClientMessage::Listen(ListenCommand::Stop));
    actor.on_client_message(ClientMessage::Listen(ListenCommand::Start {
        mode: ListenMode::Manual,
    }));
    for _ in 0..100 {
        actor.pump_workers();
        std::thread::sleep(Duration::from_millis(1));
    }

    assert!(actor.dialogue_history().is_empty());
    assert!(messages.try_recv().is_err());
}
