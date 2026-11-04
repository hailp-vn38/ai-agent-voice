use std::sync::Arc;
use std::time::Duration;

use super::harness::*;

#[test]
fn active_turn_capacity_denial_finishes_without_stt_or_history() {
    let runtime = Arc::new(AsrWorkerRuntime::new(
        Arc::new(SlowAsr),
        WorkerRuntimeConfig {
            max_workers: 2,
            voice_reserved_capacity: 1,
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
            job_submit: None,
            asr: Arc::clone(&runtime),
            vad: Arc::clone(&vad),
            llm: Arc::new(LlmRuntime::new(
                Arc::new(UnavailableLlm),
                1,
                Duration::from_secs(60),
            )),
            tts: unavailable_tts_runtime(),
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
            job_submit: None,
            asr: runtime,
            vad,
            llm: Arc::new(LlmRuntime::new(
                Arc::new(UnavailableLlm),
                1,
                Duration::from_secs(60),
            )),
            tts: unavailable_tts_runtime(),
            active_turn_limiter: limiter,
            vad_segmenter_config: VadSegmenterConfig::default(),
            pre_roll_samples: 4_800,
        },
    )
    .unwrap();
    let packet = uplink_packet();

    for actor in [&mut first, &mut second] {
        actor.on_client_message(ClientMessage::listen(ListenCommand::Start {
            mode: ListenMode::Manual,
        }));
        assert!(actor.on_binary(packet.as_bytes().to_vec()));
    }
    first.on_client_message(ClientMessage::listen(ListenCommand::Stop));
    second.on_client_message(ClientMessage::listen(ListenCommand::Stop));
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
            voice_reserved_capacity: 1,
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

    actor.on_client_message(ClientMessage::listen(ListenCommand::Start {
        mode: ListenMode::Manual,
    }));
    assert!(actor.on_binary(packet.as_bytes().to_vec()));
    actor.on_client_message(ClientMessage::listen(ListenCommand::Stop));
    actor.on_client_message(ClientMessage::Abort { session_id: None });
    for _ in 0..100 {
        actor.pump_workers();
        std::thread::sleep(Duration::from_millis(1));
    }

    assert!(actor.dialogue_history().is_empty());
    assert!(messages.try_recv().is_err());
}
