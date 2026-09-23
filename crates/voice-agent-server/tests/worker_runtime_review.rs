use std::{sync::Arc, thread, time::Duration};

use tokio::sync::mpsc;
use voice_agent_server::{
    audio::{DownlinkOpusEncoder, DownlinkPcmFrame, Pcm16Mono, PcmF32Mono},
    protocol::{ClientMessage, ListenCommand, ListenMode},
    providers::{
        AsrError, AsrEvent, AsrProvider, AsrResult, AsrSession, VadError, VadInput, VadProbability,
        VadProvider, VadSession,
    },
    session::{OutboundMessage, SessionActor, SessionPhase},
    workers::{
        AsrWorkerRuntime, VadCaptureCycleId, VadCommand, VadWorkerEvent, VadWorkerRuntime,
        WorkerIdentity, WorkerRuntimeConfig,
    },
};

struct FinalAsr;

impl AsrProvider for FinalAsr {
    fn open(&self) -> Result<Box<dyn AsrSession>, AsrError> {
        Ok(Box::new(FinalSession))
    }
}

struct FinalSession;

impl AsrSession for FinalSession {
    fn push_pcm(&mut self, _: &PcmF32Mono) -> Result<Vec<AsrEvent>, AsrError> {
        Ok(Vec::new())
    }

    fn finish(&mut self) -> Result<AsrResult, AsrError> {
        Ok(AsrResult::new("final"))
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
        thread::sleep(Duration::from_millis(50));
        Ok(AsrResult::new("late"))
    }

    fn cancel(&mut self) {}
}

struct QuietVad;

impl VadProvider for QuietVad {
    fn open(&self) -> Result<Box<dyn VadSession>, VadError> {
        Ok(Box::new(QuietVadSession))
    }

    fn adapter(&self) -> &'static str {
        "quiet"
    }
}

struct QuietVadSession;

impl VadSession for QuietVadSession {
    fn push(&mut self, input: VadInput) -> Result<VadProbability, VadError> {
        Ok(VadProbability {
            start_sample: input.start_sample,
            end_sample: input.start_sample + 512,
            probability: 0.0,
        })
    }

    fn reset(&mut self) -> Result<(), VadError> {
        Ok(())
    }

    fn close(&mut self) -> Result<(), VadError> {
        Ok(())
    }
}

struct SlowResetVad;

impl VadProvider for SlowResetVad {
    fn open(&self) -> Result<Box<dyn VadSession>, VadError> {
        Ok(Box::new(SlowResetVadSession))
    }

    fn adapter(&self) -> &'static str {
        "slow-reset"
    }
}

struct SlowResetVadSession;

impl VadSession for SlowResetVadSession {
    fn push(&mut self, input: VadInput) -> Result<VadProbability, VadError> {
        Ok(VadProbability {
            start_sample: input.start_sample,
            end_sample: input.start_sample + 512,
            probability: 0.0,
        })
    }

    fn reset(&mut self) -> Result<(), VadError> {
        thread::sleep(Duration::from_millis(50));
        Ok(())
    }

    fn close(&mut self) -> Result<(), VadError> {
        Ok(())
    }
}

fn packet() -> Vec<u8> {
    DownlinkOpusEncoder::new(65_536)
        .unwrap()
        .encode(DownlinkPcmFrame::try_new(Pcm16Mono::new(vec![0; 1_440])).unwrap())
        .unwrap()
        .as_bytes()
        .to_vec()
}

fn actor(
    session: &str,
    runtime: Arc<AsrWorkerRuntime>,
) -> (SessionActor, mpsc::Receiver<OutboundMessage>) {
    let (control_tx, control_rx) = mpsc::channel(2);
    let (audio_tx, _) = mpsc::channel(1);
    (
        SessionActor::new_with_history_and_runtime(
            session.into(),
            control_tx,
            audio_tx,
            2,
            20,
            runtime,
        )
        .unwrap(),
        control_rx,
    )
}

fn start(actor: &mut SessionActor) {
    actor.on_client_message(ClientMessage::listen(ListenCommand::Start {
        mode: ListenMode::Manual,
    }));
}

#[test]
fn worker_event_is_routed_only_to_its_own_voice_session() {
    let runtime = Arc::new(AsrWorkerRuntime::new(
        Arc::new(FinalAsr),
        WorkerRuntimeConfig {
            max_workers: 2,
            command_capacity: 8,
            final_timeout: Duration::from_secs(1),
            cleanup_grace: Duration::from_secs(1),
        },
    ));
    let (mut first, _) = actor("first", Arc::clone(&runtime));
    let (mut second, _) = actor("second", runtime);
    start(&mut first);
    start(&mut second);
    assert!(second.on_binary(packet()));
    second.on_client_message(ClientMessage::listen(ListenCommand::Stop));

    for _ in 0..50 {
        first.pump_workers();
        second.pump_workers();
        if second.phase() == SessionPhase::Ready {
            return;
        }
        thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(second.phase(), SessionPhase::Ready);
}

#[test]
fn disconnected_session_releases_capacity_after_cancel_acknowledgement() {
    let runtime = Arc::new(AsrWorkerRuntime::new(
        Arc::new(FinalAsr),
        WorkerRuntimeConfig {
            max_workers: 1,
            command_capacity: 8,
            final_timeout: Duration::from_secs(1),
            cleanup_grace: Duration::from_secs(1),
        },
    ));
    let (mut actor, _) = actor("lost", Arc::clone(&runtime));
    start(&mut actor);
    drop(actor);

    for _ in 0..50 {
        runtime.supervise_pending();
        if runtime
            .open(WorkerIdentity::new("replacement", 1, 1))
            .is_ok()
        {
            return;
        }
        thread::sleep(Duration::from_millis(1));
    }
    panic!("disconnect cleanup did not release the ASR capacity");
}

#[test]
fn final_timeout_fail_closes_the_affected_voice_session() {
    let runtime = Arc::new(AsrWorkerRuntime::new(
        Arc::new(SlowAsr),
        WorkerRuntimeConfig {
            max_workers: 1,
            command_capacity: 8,
            final_timeout: Duration::from_millis(1),
            cleanup_grace: Duration::from_millis(1),
        },
    ));
    let (mut actor, mut control_rx) = actor("timed-out", runtime);
    start(&mut actor);
    assert!(actor.on_binary(packet()));
    actor.on_client_message(ClientMessage::listen(ListenCommand::Stop));

    for _ in 0..50 {
        actor.pump_workers();
        if matches!(control_rx.try_recv(), Ok(OutboundMessage::Close(1011))) {
            return;
        }
        thread::sleep(Duration::from_millis(1));
    }
    panic!("ASR final timeout did not close the affected session with 1011");
}

#[test]
fn vad_events_are_routed_to_the_registered_session_mailbox() {
    let runtime = VadWorkerRuntime::new(
        Arc::new(QuietVad),
        WorkerRuntimeConfig {
            max_workers: 2,
            command_capacity: 8,
            final_timeout: Duration::from_secs(1),
            cleanup_grace: Duration::from_secs(1),
        },
    );
    let mut first_events = runtime.register_session("first");
    let mut second_events = runtime.register_session("second");
    runtime.open(WorkerIdentity::new("first", 1, 1)).unwrap();
    runtime.open(WorkerIdentity::new("second", 1, 1)).unwrap();

    let mut first_opened = false;
    let mut second_opened = false;
    for _ in 0..50 {
        runtime.supervise_pending();
        first_opened |= matches!(
            first_events.try_recv().ok(),
            Some(VadWorkerEvent::Opened { ref identity }) if identity.session() == "first"
        );
        second_opened |= matches!(
            second_events.try_recv().ok(),
            Some(VadWorkerEvent::Opened { ref identity }) if identity.session() == "second"
        );
        if first_opened && second_opened {
            return;
        }
        thread::sleep(Duration::from_millis(1));
    }
    panic!("VAD events were not routed to their owning sessions");
}

#[test]
fn disconnected_vad_session_releases_capacity_after_close_acknowledgement() {
    let runtime = VadWorkerRuntime::new(
        Arc::new(QuietVad),
        WorkerRuntimeConfig {
            max_workers: 1,
            command_capacity: 8,
            final_timeout: Duration::from_secs(1),
            cleanup_grace: Duration::from_secs(1),
        },
    );
    let lease = runtime.open(WorkerIdentity::new("lost", 1, 1)).unwrap();
    runtime.register_session("lost");
    runtime.send(lease, VadCommand::Close).unwrap();
    runtime.unregister_session("lost");

    for _ in 0..50 {
        runtime.supervise_pending();
        if runtime
            .open(WorkerIdentity::new("replacement", 1, 1))
            .is_ok()
        {
            return;
        }
        thread::sleep(Duration::from_millis(1));
    }
    panic!("VAD close acknowledgement did not release capacity after disconnect");
}

#[test]
fn vad_reset_timeout_quarantines_the_worker_and_routes_the_fatal_event() {
    let runtime = VadWorkerRuntime::new(
        Arc::new(SlowResetVad),
        WorkerRuntimeConfig {
            max_workers: 1,
            command_capacity: 8,
            final_timeout: Duration::from_millis(1),
            cleanup_grace: Duration::from_millis(1),
        },
    );
    let mut events = runtime.register_session("timed-out");
    let lease = runtime
        .open(WorkerIdentity::new("timed-out", 1, 1))
        .unwrap();
    runtime
        .send(
            lease,
            VadCommand::Reset {
                cycle: VadCaptureCycleId::new(1),
            },
        )
        .unwrap();

    for _ in 0..50 {
        runtime.supervise_pending();
        if matches!(events.try_recv(), Ok(VadWorkerEvent::ResetTimedOut { .. })) {
            assert!(
                runtime
                    .open(WorkerIdentity::new("replacement", 1, 1))
                    .is_err()
            );
            return;
        }
        thread::sleep(Duration::from_millis(1));
    }
    panic!("VAD reset timeout did not quarantine its worker");
}
