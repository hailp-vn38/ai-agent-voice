use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

use super::harness::*;

#[test]
fn auto_cycle_opens_asr_after_speech_start_and_rearms_only_after_reset_done() {
    let (control, mut messages) = mpsc::channel(2);
    let (audio, _) = mpsc::channel(1);
    let config = WorkerRuntimeConfig {
        max_workers: 1,
        voice_reserved_capacity: 1,
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
            job_submit: None,
            asr,
            vad,
            llm: Arc::new(LlmRuntime::new(
                Arc::new(UnavailableLlm),
                1,
                Duration::from_secs(60),
            )),
            tts: unavailable_tts_runtime(),
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

    actor.on_client_message(ClientMessage::listen(ListenCommand::Start {
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
    assert!(matches!(messages.try_recv(), Ok(message) if message
        .as_text()
        .is_some_and(|text| text.contains("auto final"))));
    // This is the next microphone frame after the turn has completed (the same re-arm
    // boundary used after a delivered TTS response). ResetDone must make the pinned VAD
    // lease usable again rather than closing the WebSocket on its first Push.
    assert!(actor.on_binary(packet.as_bytes().to_vec()));
    assert_eq!(actor.phase(), SessionPhase::Listening);
}

#[test]
fn auto_retains_pre_roll_and_all_pcm_arrived_while_vad_events_lag() {
    let (control, _) = mpsc::channel(2);
    let (audio, _) = mpsc::channel(1);
    let config = WorkerRuntimeConfig {
        max_workers: 1,
        voice_reserved_capacity: 1,
        command_capacity: 8,
        final_timeout: Duration::from_secs(1),
        cleanup_grace: Duration::from_secs(1),
    };
    let pushed_samples = Arc::new(AtomicUsize::new(0));
    let mut actor = SessionActor::new_with_runtimes_and_limiter(
        "retention".into(),
        control,
        audio,
        4,
        20,
        SessionRuntimes {
            job_submit: None,
            asr: Arc::new(AsrWorkerRuntime::new(
                Arc::new(RecordingAsr {
                    pushed_samples: Arc::clone(&pushed_samples),
                }),
                config.clone(),
            )),
            vad: Arc::new(VadWorkerRuntime::new(Arc::new(DelayedOnsetVad), config)),
            llm: Arc::new(LlmRuntime::new(
                Arc::new(UnavailableLlm),
                1,
                Duration::from_secs(60),
            )),
            tts: unavailable_tts_runtime(),
            active_turn_limiter: Arc::new(ActiveTurnLimiter::new(1)),
            vad_segmenter_config: VadSegmenterConfig {
                speech_threshold: 0.5,
                exit_threshold: 0.35,
                min_speech_samples: 512,
                end_silence_samples: 9_600,
            },
            pre_roll_samples: 480,
        },
    )
    .unwrap();
    let packet = uplink_packet();

    actor.on_client_message(ClientMessage::listen(ListenCommand::Start {
        mode: ListenMode::Auto,
    }));
    wait_for_phase(&mut actor, SessionPhase::Listening);
    for _ in 0..5 {
        assert!(actor.on_binary(packet.as_bytes().to_vec()));
    }
    for _ in 0..100 {
        actor.pump_workers();
        if pushed_samples.load(Ordering::Relaxed) != 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }

    // The second 512-sample VAD window confirms onset at 512; pre-roll begins at 32.
    // All five 960-sample frames arrived before the delayed semantic event, so ASR receives
    // the complete retained range [32, 4_800).
    assert_eq!(pushed_samples.load(Ordering::Relaxed), 4_768);
}

#[test]
fn stale_vad_probability_before_reset_does_not_open_asr_for_the_new_cycle() {
    let (control, _) = mpsc::channel(2);
    let (audio, _) = mpsc::channel(1);
    let config = WorkerRuntimeConfig {
        max_workers: 1,
        voice_reserved_capacity: 1,
        command_capacity: 8,
        final_timeout: Duration::from_secs(1),
        cleanup_grace: Duration::from_secs(1),
    };
    let pushed_samples = Arc::new(AtomicUsize::new(0));
    let mut actor = SessionActor::new_with_runtimes_and_limiter(
        "stale-cycle".into(),
        control,
        audio,
        4,
        20,
        SessionRuntimes {
            job_submit: None,
            asr: Arc::new(AsrWorkerRuntime::new(
                Arc::new(RecordingAsr {
                    pushed_samples: Arc::clone(&pushed_samples),
                }),
                config.clone(),
            )),
            vad: Arc::new(VadWorkerRuntime::new(Arc::new(DelayedOnsetVad), config)),
            llm: Arc::new(LlmRuntime::new(
                Arc::new(UnavailableLlm),
                1,
                Duration::from_secs(60),
            )),
            tts: unavailable_tts_runtime(),
            active_turn_limiter: Arc::new(ActiveTurnLimiter::new(1)),
            vad_segmenter_config: VadSegmenterConfig {
                speech_threshold: 0.5,
                exit_threshold: 0.35,
                min_speech_samples: 512,
                end_silence_samples: 9_600,
            },
            pre_roll_samples: 480,
        },
    )
    .unwrap();
    let packet = uplink_packet();

    actor.on_client_message(ClientMessage::listen(ListenCommand::Start {
        mode: ListenMode::Auto,
    }));
    wait_for_phase(&mut actor, SessionPhase::Listening);
    assert!(actor.on_binary(packet.as_bytes().to_vec()));
    assert!(actor.on_binary(packet.as_bytes().to_vec()));
    // Do not pump between the audio and reset: the old cycle's probability is now delayed
    // in the actor mailbox, behind the semantic reset boundary.
    actor.on_client_message(ClientMessage::listen(ListenCommand::Start {
        mode: ListenMode::Auto,
    }));
    wait_for_phase(&mut actor, SessionPhase::Listening);

    assert_eq!(pushed_samples.load(Ordering::Relaxed), 0);
    assert!(actor.dialogue_history().is_empty());
}
