use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

use super::harness::*;

#[test]
fn repeated_auto_start_during_speech_preserves_asr_pcm() {
    let pushed_samples = Arc::new(AtomicUsize::new(0));
    let mut actor = auto_actor_for_duplicate_start(
        Arc::new(RecordingAsr {
            pushed_samples: Arc::clone(&pushed_samples),
        }),
        Arc::new(DelayedOnsetVad),
    );
    let packet = uplink_packet();
    actor.on_client_message(ClientMessage::listen(ListenCommand::Start {
        mode: ListenMode::Auto,
    }));
    wait_for_phase(&mut actor, SessionPhase::Listening);
    for _ in 0..2 {
        assert!(actor.on_binary(packet.as_bytes().to_vec()));
    }
    for _ in 0..100 {
        actor.pump_workers();
        if pushed_samples.load(Ordering::Relaxed) > 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let before = pushed_samples.load(Ordering::Relaxed);
    assert!(before > 0, "SpeechStart must open and feed ASR");

    actor.on_client_message(ClientMessage::listen(ListenCommand::Start {
        mode: ListenMode::Auto,
    }));
    assert!(actor.on_binary(packet.as_bytes().to_vec()));
    for _ in 0..100 {
        actor.pump_workers();
        if pushed_samples.load(Ordering::Relaxed) > before {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        pushed_samples.load(Ordering::Relaxed) > before,
        "PCM after duplicate listen:start must reach the same ASR stream"
    );
}

#[test]
fn repeated_auto_start_during_asr_finalization_preserves_final_text() {
    let mut actor = auto_actor_for_duplicate_start(
        Arc::new(FakeAsr {
            final_text: "complete utterance".into(),
        }),
        Arc::new(BoundaryVad),
    );
    let packet = uplink_packet();
    actor.on_client_message(ClientMessage::listen(ListenCommand::Start {
        mode: ListenMode::Auto,
    }));
    wait_for_phase(&mut actor, SessionPhase::Listening);
    assert!(actor.on_binary(packet.as_bytes().to_vec()));
    assert!(actor.on_binary(packet.as_bytes().to_vec()));
    wait_for_phase(&mut actor, SessionPhase::Processing);

    // Finish has been sent by SpeechEnd, but its Final has not been consumed yet.
    actor.on_client_message(ClientMessage::listen(ListenCommand::Start {
        mode: ListenMode::Auto,
    }));
    for _ in 0..100 {
        actor.pump_workers();
        if !actor.dialogue_history().is_empty() {
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(actor.dialogue_history(), &["complete utterance"]);
}

#[test]
fn repeated_auto_start_rearms_the_pinned_vad_lease_with_one_worker_slot() {
    let opens = Arc::new(AtomicUsize::new(0));
    let resets = Arc::new(AtomicUsize::new(0));
    let closes = Arc::new(AtomicUsize::new(0));
    let vad = Arc::new(VadWorkerRuntime::new(
        Arc::new(CountingVad {
            opens: Arc::clone(&opens),
            resets: Arc::clone(&resets),
            closes: Arc::clone(&closes),
        }),
        WorkerRuntimeConfig {
            max_workers: 1,
            voice_reserved_capacity: 1,
            command_capacity: 8,
            final_timeout: Duration::from_secs(1),
            cleanup_grace: Duration::from_secs(1),
        },
    ));
    let mut actor = auto_actor_with_counting_vad(vad);

    actor.on_client_message(ClientMessage::listen(ListenCommand::Start {
        mode: ListenMode::Auto,
    }));
    wait_for_phase(&mut actor, SessionPhase::Listening);
    actor.on_client_message(ClientMessage::listen(ListenCommand::Start {
        mode: ListenMode::Auto,
    }));
    // A duplicate from the client while Reset is in flight must be idempotent.
    actor.on_client_message(ClientMessage::listen(ListenCommand::Start {
        mode: ListenMode::Auto,
    }));
    wait_for_phase(&mut actor, SessionPhase::Listening);

    assert_eq!(opens.load(Ordering::Relaxed), 1);
    assert_eq!(resets.load(Ordering::Relaxed), 1);
    assert_eq!(closes.load(Ordering::Relaxed), 0);
}

#[test]
fn repeated_realtime_start_rearms_the_pinned_vad_lease_with_one_worker_slot() {
    let opens = Arc::new(AtomicUsize::new(0));
    let resets = Arc::new(AtomicUsize::new(0));
    let closes = Arc::new(AtomicUsize::new(0));
    let vad = Arc::new(VadWorkerRuntime::new(
        Arc::new(CountingVad {
            opens: Arc::clone(&opens),
            resets: Arc::clone(&resets),
            closes: Arc::clone(&closes),
        }),
        WorkerRuntimeConfig {
            max_workers: 1,
            voice_reserved_capacity: 1,
            command_capacity: 8,
            final_timeout: Duration::from_secs(1),
            cleanup_grace: Duration::from_secs(1),
        },
    ));
    let mut actor = auto_actor_with_counting_vad(vad);

    actor.on_client_message(ClientMessage::listen(ListenCommand::Start {
        mode: ListenMode::Realtime,
    }));
    wait_for_phase(&mut actor, SessionPhase::Listening);
    actor.on_client_message(ClientMessage::listen(ListenCommand::Start {
        mode: ListenMode::Realtime,
    }));
    wait_for_phase(&mut actor, SessionPhase::Listening);

    assert_eq!(opens.load(Ordering::Relaxed), 1);
    assert_eq!(resets.load(Ordering::Relaxed), 1);
    assert_eq!(closes.load(Ordering::Relaxed), 0);
}

#[test]
fn abort_in_auto_rearms_the_pinned_vad_lease_instead_of_closing_it() {
    let opens = Arc::new(AtomicUsize::new(0));
    let resets = Arc::new(AtomicUsize::new(0));
    let closes = Arc::new(AtomicUsize::new(0));
    let vad = Arc::new(VadWorkerRuntime::new(
        Arc::new(CountingVad {
            opens: Arc::clone(&opens),
            resets: Arc::clone(&resets),
            closes: Arc::clone(&closes),
        }),
        WorkerRuntimeConfig {
            max_workers: 1,
            voice_reserved_capacity: 1,
            command_capacity: 8,
            final_timeout: Duration::from_secs(1),
            cleanup_grace: Duration::from_secs(1),
        },
    ));
    let mut actor = auto_actor_with_counting_vad(vad);

    actor.on_client_message(ClientMessage::listen(ListenCommand::Start {
        mode: ListenMode::Auto,
    }));
    wait_for_phase(&mut actor, SessionPhase::Listening);
    actor.on_client_message(ClientMessage::Abort { session_id: None });
    wait_for_phase(&mut actor, SessionPhase::Listening);
    actor.on_client_message(ClientMessage::listen(ListenCommand::Start {
        mode: ListenMode::Auto,
    }));
    wait_for_phase(&mut actor, SessionPhase::Listening);

    assert_eq!(opens.load(Ordering::Relaxed), 1);
    assert_eq!(resets.load(Ordering::Relaxed), 2);
    assert_eq!(closes.load(Ordering::Relaxed), 0);
}
