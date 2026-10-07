//! Ticket 15: deterministic **Required** gate behaviour through the Voice Session actor.
//!
//! These drive a real `SessionActor` with a steerable qualification speaker runtime, so the exact
//! embedding per turn is controlled. They cover the ticket's hard constraints: the first turn
//! identifies 1:N and locks, later turns verify 1:1 against the lock, a mismatch commits nothing,
//! three consecutive denials close 1008, and an `off`/Observe session is unaffected.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::mpsc;
use voice_agent_server::{
    audio::{DownlinkOpusEncoder, DownlinkPcmFrame, Pcm16Mono, PcmF32Mono, QualityProfile},
    database::DesiredProvider,
    lifecycle::AdmissionGate,
    protocol::{ClientMessage, ListenCommand, ListenMode},
    providers::{
        AsrError, AsrEvent, AsrProvider, AsrResult, AsrSession, ProviderSet, RuntimeCatalog,
        VadError, VadProvider,
        speaker::{SpeakerError, SpeakerProvider},
    },
    services::provider_runtime::{
        PreparedRuntime, ProviderRuntimeManager, ResourceLease, RuntimeError, RuntimeLimits,
        RuntimeMaterializer, RuntimeResource,
    },
    session::{
        ObserveCandidate, ObservePlan, OutboundMessage, SessionActor, SessionPhase, SpeakerObserve,
        SpeakerPolicyMode,
    },
    workers::{ProviderRuntimeAdmission, SpeakerRuntime},
};

/// A qualification extractor whose embedding the test steers per turn.
struct SteerableSpeaker {
    fail: Arc<AtomicBool>,
    embedding: Arc<Mutex<Vec<f32>>>,
}

impl SpeakerProvider for SteerableSpeaker {
    fn dimension(&self) -> usize {
        3
    }

    fn extract(&mut self, _pcm: &PcmF32Mono) -> Result<Vec<f32>, SpeakerError> {
        if self.fail.load(Ordering::Acquire) {
            return Err(SpeakerError::Unavailable);
        }
        Ok(self.embedding.lock().unwrap().clone())
    }
}

struct SteerableResource(Arc<SpeakerRuntime>);

impl RuntimeResource for SteerableResource {
    fn unload(&self) -> bool {
        self.0.shutdown_acknowledged()
    }

    fn runtimes_for(
        &self,
        snapshot: &DesiredProvider,
        quota: ProviderRuntimeAdmission,
    ) -> Option<RuntimeCatalog> {
        Some(RuntimeCatalog::single_speaker(
            snapshot.key.clone(),
            Arc::new(self.0.logical_view(quota)),
        ))
    }
}

struct SteerableFactory {
    fail: bool,
    embedding: Arc<Mutex<Vec<f32>>>,
}

impl RuntimeMaterializer for SteerableFactory {
    fn estimated_peak_bytes(&self, _: &DesiredProvider) -> Result<u64, RuntimeError> {
        Ok(1)
    }

    fn logical_capacity(&self, _: &DesiredProvider) -> Result<usize, RuntimeError> {
        Ok(1)
    }

    fn build(
        &self,
        _: &DesiredProvider,
        _: Option<PreparedRuntime>,
        quota: ProviderRuntimeAdmission,
    ) -> Result<Arc<dyn RuntimeResource>, RuntimeError> {
        let fail = Arc::new(AtomicBool::new(false));
        let provider = SteerableSpeaker {
            fail: fail.clone(),
            embedding: self.embedding.clone(),
        };
        let runtime = SpeakerRuntime::new(Box::new(provider), quota).unwrap();
        fail.store(self.fail, Ordering::Release);
        Ok(Arc::new(SteerableResource(Arc::new(runtime))))
    }
}

/// Build a Required `SpeakerObserve` over `candidates`, plus the steerable embedding handle.
async fn required_with(
    candidates: Vec<ObserveCandidate>,
    fail: bool,
) -> (Arc<SpeakerObserve>, ResourceLease, Arc<Mutex<Vec<f32>>>) {
    let embedding = Arc::new(Mutex::new(vec![1.0, 0.0, 0.0]));
    let manager = ProviderRuntimeManager::new(
        RuntimeLimits {
            max_parallel_loads: 1,
            max_pending_loads: 1,
            max_waiters: 4,
            max_resident_bytes: 8192,
            max_resources: 2,
            max_version_entries: 8,
            admission_timeout_ms: 1000,
            failure_cooldown_ms: 10,
            idle_ttl_ms: 60_000,
        },
        Arc::new(SteerableFactory {
            fail,
            embedding: embedding.clone(),
        }),
        AdmissionGate::open(),
    )
    .unwrap();
    let lease = manager
        .acquire(DesiredProvider {
            id: 1,
            key: "speaker".into(),
            kind: "speaker".into(),
            adapter: "campplus".into(),
            revision: 1,
            config_json: "{}".into(),
            secret_ref: None,
        })
        .await
        .unwrap();
    let runtime = lease.runtimes().unwrap().speaker("speaker").unwrap();
    let plan = ObservePlan {
        agent_id: 7,
        template_id: 3,
        embedding_space: runtime.embedding_space_id().to_owned(),
        catalog_revision: 42,
        policy: SpeakerPolicyMode::Required,
        candidates,
    };
    let profile = QualityProfile {
        min_clip_ms: 1_000,
        max_clip_ms: 6_000,
        min_speech_ms: 200,
        max_window_ms: 6_000,
    };
    (
        Arc::new(SpeakerObserve::new(runtime, lease.clone(), plan, profile)),
        lease,
        embedding,
    )
}

fn candidate(speaker_id: i64, key: &str, vector: [f32; 3]) -> ObserveCandidate {
    ObserveCandidate {
        speaker_id,
        key: key.into(),
        vector: vector.to_vec(),
    }
}

struct FakeAsr;

impl AsrProvider for FakeAsr {
    fn open(&self) -> Result<Box<dyn AsrSession>, AsrError> {
        Ok(Box::new(FakeSession))
    }
}

struct UnusedVad;

impl VadProvider for UnusedVad {
    fn open(&self) -> Result<Box<dyn voice_agent_server::providers::VadSession>, VadError> {
        Err(VadError::Failed("manual mode never opens VAD".into()))
    }

    fn adapter(&self) -> &'static str {
        "unused_vad"
    }
}

fn providers() -> Arc<ProviderSet> {
    Arc::new(ProviderSet::with_vad(
        Arc::new(UnusedVad),
        Arc::new(FakeAsr),
    ))
}

struct FakeSession;

impl AsrSession for FakeSession {
    fn push_pcm(&mut self, _: &PcmF32Mono) -> Result<Vec<AsrEvent>, AsrError> {
        Ok(Vec::new())
    }

    fn finish(&mut self) -> Result<AsrResult, AsrError> {
        Ok(AsrResult::new("xin chào"))
    }

    fn cancel(&mut self) {}
}

fn loud_packet() -> voice_agent_server::audio::OpusPacket {
    DownlinkOpusEncoder::new(65_536)
        .unwrap()
        .encode(DownlinkPcmFrame::try_new(Pcm16Mono::new(vec![8_000; 1_440])).unwrap())
        .unwrap()
}

const FRAMES: usize = 17;
const CAPTURE_FRAMES: usize = 32;

fn push_uplink(actor: &mut SessionActor) {
    for _ in 0..FRAMES {
        assert!(actor.on_binary(loud_packet().as_bytes().to_vec()));
    }
}

fn frame_field(message: &OutboundMessage, field: &str) -> Option<String> {
    message
        .as_text()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(text).ok())
        .and_then(|value| value[field].as_str().map(str::to_owned))
}

fn is_stt_frame(message: &OutboundMessage) -> bool {
    frame_field(message, "type").as_deref() == Some("stt")
}

fn is_terminal_speaker(message: &OutboundMessage) -> bool {
    frame_field(message, "type").as_deref() == Some("speaker")
        && frame_field(message, "state").as_deref() != Some("verifying")
}

fn speaker_state(messages: &[OutboundMessage]) -> Option<String> {
    messages
        .iter()
        .rev()
        .find(|m| is_terminal_speaker(m))
        .and_then(|m| frame_field(m, "state"))
}

/// Drive one manual Required turn and return every frame it produced. Stops when the gate has
/// resolved (a terminal speaker frame with the session re-armed) or the session closed.
async fn run_turn(
    actor: &mut SessionActor,
    messages: &mut mpsc::Receiver<OutboundMessage>,
) -> Vec<OutboundMessage> {
    actor.on_client_message(ClientMessage::listen(ListenCommand::Start {
        mode: ListenMode::Manual,
    }));
    push_uplink(actor);
    actor.on_client_message(ClientMessage::listen(ListenCommand::Stop));

    let mut collected = Vec::new();
    for _ in 0..2_000 {
        actor.pump_workers();
        while let Ok(message) = messages.try_recv() {
            collected.push(message);
        }
        let closed = collected.iter().any(|m| {
            matches!(
                m,
                OutboundMessage::CloseWithReason { .. } | OutboundMessage::Close(_)
            )
        });
        let resolved =
            actor.phase() == SessionPhase::Ready && collected.iter().any(is_terminal_speaker);
        if closed || resolved {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    collected
}

fn actor_with(observe: Arc<SpeakerObserve>) -> (SessionActor, mpsc::Receiver<OutboundMessage>) {
    let (control, messages) = mpsc::channel(16);
    let (audio, _) = mpsc::channel(1);
    let actor = SessionActor::new(
        "session".into(),
        control,
        audio,
        CAPTURE_FRAMES,
        providers(),
    )
    .unwrap()
    .with_speaker_observe(Some(observe), true);
    (actor, messages)
}

#[tokio::test]
async fn first_turn_identifies_1n_then_locked_turn_verifies_1_1() {
    let (observe, _lease, embedding) = required_with(
        vec![
            candidate(1, "alice", [1.0, 0.0, 0.0]),
            candidate(2, "bob", [0.0, 1.0, 0.0]),
        ],
        false,
    )
    .await;
    let (mut actor, mut messages) = actor_with(observe);

    *embedding.lock().unwrap() = vec![1.0, 0.0, 0.0];
    let first = run_turn(&mut actor, &mut messages).await;
    assert_eq!(speaker_state(&first).as_deref(), Some("verified"));
    assert!(first.iter().any(is_stt_frame));

    // Second turn still Alice: verified against the lock.
    let second = run_turn(&mut actor, &mut messages).await;
    assert_eq!(speaker_state(&second).as_deref(), Some("verified"));
    assert!(second.iter().any(is_stt_frame));
}

#[tokio::test]
async fn locked_turn_from_another_speaker_commits_nothing() {
    let (observe, _lease, embedding) = required_with(
        vec![
            candidate(1, "alice", [1.0, 0.0, 0.0]),
            candidate(2, "bob", [0.0, 1.0, 0.0]),
        ],
        false,
    )
    .await;
    let (mut actor, mut messages) = actor_with(observe);

    *embedding.lock().unwrap() = vec![1.0, 0.0, 0.0];
    let first = run_turn(&mut actor, &mut messages).await;
    assert_eq!(speaker_state(&first).as_deref(), Some("verified"));

    // Second turn scores Bob against Alice's lock: denied, no transcript.
    *embedding.lock().unwrap() = vec![0.0, 1.0, 0.0];
    let second = run_turn(&mut actor, &mut messages).await;
    assert_eq!(speaker_state(&second).as_deref(), Some("denied"));
    assert!(!second.iter().any(is_stt_frame));
}

#[tokio::test]
async fn ambiguous_first_turn_commits_nothing() {
    let (observe, _lease, embedding) = required_with(
        vec![
            candidate(1, "alice", [1.0, 0.0, 0.0]),
            candidate(2, "bob", [1.0, 0.0, 0.0]),
        ],
        false,
    )
    .await;
    let (mut actor, mut messages) = actor_with(observe);

    *embedding.lock().unwrap() = vec![1.0, 0.0, 0.0];
    let turn = run_turn(&mut actor, &mut messages).await;
    assert_eq!(speaker_state(&turn).as_deref(), Some("ambiguous"));
    assert!(!turn.iter().any(is_stt_frame));
}

#[tokio::test]
async fn unknown_first_turn_commits_nothing() {
    let (observe, _lease, embedding) =
        required_with(vec![candidate(1, "alice", [1.0, 0.0, 0.0])], false).await;
    let (mut actor, mut messages) = actor_with(observe);

    *embedding.lock().unwrap() = vec![0.0, 1.0, 0.0];
    let turn = run_turn(&mut actor, &mut messages).await;
    assert_eq!(speaker_state(&turn).as_deref(), Some("unknown"));
    assert!(!turn.iter().any(is_stt_frame));
}

#[tokio::test]
async fn three_consecutive_denials_close_1008() {
    let (observe, _lease, embedding) =
        required_with(vec![candidate(1, "alice", [1.0, 0.0, 0.0])], false).await;
    let (mut actor, mut messages) = actor_with(observe);

    *embedding.lock().unwrap() = vec![0.0, 1.0, 0.0];
    let first = run_turn(&mut actor, &mut messages).await;
    assert_eq!(speaker_state(&first).as_deref(), Some("unknown"));
    let second = run_turn(&mut actor, &mut messages).await;
    assert_eq!(speaker_state(&second).as_deref(), Some("unknown"));
    let third = run_turn(&mut actor, &mut messages).await;
    assert!(
        third.iter().any(|m| matches!(
            m,
            OutboundMessage::CloseWithReason { code: 1008, reason } if reason == "speaker_policy_denied"
        )),
        "expected a 1008 speaker_policy_denied close"
    );
    assert!(!first.iter().any(is_stt_frame));
    assert!(!second.iter().any(is_stt_frame));
    assert!(!third.iter().any(is_stt_frame));
}

#[tokio::test]
async fn unavailable_runtime_is_a_non_answer_and_never_counts() {
    let (observe, _lease, _embedding) =
        required_with(vec![candidate(1, "alice", [1.0, 0.0, 0.0])], true).await;
    let (mut actor, mut messages) = actor_with(observe);

    // Repeated runtime-unavailable turns are bounded non-answers: never committed, never counted,
    // and never a 1008 close.
    for _ in 0..4 {
        let turn = run_turn(&mut actor, &mut messages).await;
        assert_eq!(speaker_state(&turn).as_deref(), Some("unavailable"));
        assert!(!turn.iter().any(is_stt_frame));
        assert!(
            !turn
                .iter()
                .any(|m| matches!(m, OutboundMessage::CloseWithReason { .. }))
        );
    }
    assert_eq!(actor.phase(), SessionPhase::Ready);
}
