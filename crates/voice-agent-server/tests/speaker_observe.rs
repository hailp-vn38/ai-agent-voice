//! Ticket 10: deterministic Observe behaviour through the Voice Session actor.
//!
//! These drive a real `SessionActor` with a real (qualification) `SpeakerRuntime` obtained from a
//! real provider-runtime lease, so the Observe path is exercised end to end without native models
//! or a network. The assertions cover the ticket's hard constraints: bounded state on the wire,
//! no identity or score leaked, `off` doing no work, and an unavailable runtime never blocking the
//! core transcript path.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
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

/// A qualification extractor with a fixed embedding, so scoring is deterministic.
struct FixedSpeaker {
    /// Flipped after warmup so the runtime loads, then fails every real extraction.
    fail: Arc<AtomicBool>,
}

impl SpeakerProvider for FixedSpeaker {
    fn dimension(&self) -> usize {
        3
    }

    fn extract(&mut self, _pcm: &PcmF32Mono) -> Result<Vec<f32>, SpeakerError> {
        if self.fail.load(Ordering::Acquire) {
            return Err(SpeakerError::Unavailable);
        }
        Ok(vec![1.0, 0.0, 0.0])
    }
}

struct FixedSpeakerResource(Arc<SpeakerRuntime>);

impl RuntimeResource for FixedSpeakerResource {
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

struct FixedSpeakerFactory {
    fail: bool,
}

impl RuntimeMaterializer for FixedSpeakerFactory {
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
        let provider = FixedSpeaker { fail: fail.clone() };
        let runtime = SpeakerRuntime::new(Box::new(provider), quota).unwrap();
        // Load succeeded (warmup); now make every real extraction unavailable.
        fail.store(self.fail, Ordering::Release);
        Ok(Arc::new(FixedSpeakerResource(Arc::new(runtime))))
    }
}

async fn observe_with(fail: bool) -> (Arc<SpeakerObserve>, ResourceLease) {
    observe_with_policy(fail, SpeakerPolicyMode::Observe).await
}

async fn observe_with_policy(
    fail: bool,
    policy: SpeakerPolicyMode,
) -> (Arc<SpeakerObserve>, ResourceLease) {
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
        Arc::new(FixedSpeakerFactory { fail }),
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
        policy,
        candidates: vec![ObserveCandidate {
            speaker_id: 1,
            key: "alice".into(),
            vector: vec![1.0, 0.0, 0.0],
        }],
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
    )
}

struct FakeAsr;

impl AsrProvider for FakeAsr {
    fn open(&self) -> Result<Box<dyn AsrSession>, AsrError> {
        Ok(Box::new(FakeSession))
    }
}

/// The actor requires a VAD provider to be present even for manual sessions; opening it is only
/// rejected because manual mode never holds a VAD lease.
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

/// Non-silent uplink (well above the quality gate's speech RMS) so Observe sees speech.
fn loud_packet() -> voice_agent_server::audio::OpusPacket {
    DownlinkOpusEncoder::new(65_536)
        .unwrap()
        .encode(DownlinkPcmFrame::try_new(Pcm16Mono::new(vec![8_000; 1_440])).unwrap())
        .unwrap()
}

/// One second of uplink at 960 samples per decoded frame: the extractor's minimum window.
const FRAMES: usize = 17;
/// Frame budget comfortably above [`FRAMES`].
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

fn is_speaker_frame(message: &OutboundMessage) -> bool {
    frame_field(message, "type").as_deref() == Some("speaker")
}

/// A completed Observe result; the intermediate `verifying` frame does not count.
fn is_terminal_speaker(message: &OutboundMessage) -> bool {
    is_speaker_frame(message) && frame_field(message, "state").as_deref() != Some("verifying")
}

fn is_stt_frame(message: &OutboundMessage) -> bool {
    frame_field(message, "type").as_deref() == Some("stt")
}

/// Pump until the core transcript has committed and, when Observe is on, its result has landed.
async fn drain(
    actor: &mut SessionActor,
    messages: &mut mpsc::Receiver<OutboundMessage>,
    expect_terminal_speaker: bool,
) -> Vec<OutboundMessage> {
    let mut collected = Vec::new();
    for _ in 0..2_000 {
        actor.pump_workers();
        while let Ok(message) = messages.try_recv() {
            collected.push(message);
        }
        let ready = actor.phase() == SessionPhase::Ready;
        let done = ready
            && collected.iter().any(is_stt_frame)
            && (!expect_terminal_speaker || collected.iter().any(is_terminal_speaker));
        if done {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    collected
}

#[tokio::test]
async fn observe_emits_bounded_state_without_identity_or_score() {
    let (observe, _lease) = observe_with(false).await;
    let (control, mut messages) = mpsc::channel(8);
    let (audio, _) = mpsc::channel(1);
    let providers = providers();
    let mut actor = SessionActor::new("session".into(), control, audio, CAPTURE_FRAMES, providers)
        .unwrap()
        .with_speaker_observe(Some(observe), true);

    actor.on_client_message(ClientMessage::listen(ListenCommand::Start {
        mode: ListenMode::Manual,
    }));
    push_uplink(&mut actor);
    actor.on_client_message(ClientMessage::listen(ListenCommand::Stop));
    let messages = drain(&mut actor, &mut messages, true).await;

    let speaker = messages
        .iter()
        .filter_map(|m| m.as_text())
        .map(|t| serde_json::from_str::<serde_json::Value>(t).unwrap())
        .find(|v| v["type"] == "speaker" && v["state"] != "verifying")
        .expect("an observe state frame");
    assert_eq!(speaker["state"], "verified");
    assert_eq!(
        speaker.as_object().unwrap().len(),
        2,
        "frame leaked fields: {speaker}"
    );
    let text = speaker.to_string();
    for forbidden in ["alice", "score", "speaker_id", "0.9", "embedding"] {
        assert!(
            !text.contains(forbidden),
            "frame leaked {forbidden}: {text}"
        );
    }
}

#[tokio::test]
async fn observe_off_emits_no_speaker_frame_and_retains_nothing() {
    let (control, mut messages) = mpsc::channel(8);
    let (audio, _) = mpsc::channel(1);
    let providers = providers();
    let mut actor = SessionActor::new("session".into(), control, audio, CAPTURE_FRAMES, providers)
        .unwrap()
        .with_speaker_observe(None, true);

    actor.on_client_message(ClientMessage::listen(ListenCommand::Start {
        mode: ListenMode::Manual,
    }));
    push_uplink(&mut actor);
    actor.on_client_message(ClientMessage::listen(ListenCommand::Stop));
    let messages = drain(&mut actor, &mut messages, false).await;

    assert!(messages.iter().any(is_stt_frame));
    assert!(!messages.iter().any(is_speaker_frame));
}

#[tokio::test]
async fn observe_unavailable_runtime_does_not_block_core_path() {
    let (observe, _lease) = observe_with(true).await;
    let (control, mut messages) = mpsc::channel(8);
    let (audio, _) = mpsc::channel(1);
    let providers = providers();
    let mut actor = SessionActor::new("session".into(), control, audio, CAPTURE_FRAMES, providers)
        .unwrap()
        .with_speaker_observe(Some(observe), true);

    actor.on_client_message(ClientMessage::listen(ListenCommand::Start {
        mode: ListenMode::Manual,
    }));
    push_uplink(&mut actor);
    actor.on_client_message(ClientMessage::listen(ListenCommand::Stop));
    let messages = drain(&mut actor, &mut messages, true).await;

    // The core transcript still committed...
    assert!(messages.iter().any(is_stt_frame));
    // ...and Observe reported `unavailable` rather than failing the turn.
    let speaker = messages
        .iter()
        .filter_map(|m| m.as_text())
        .map(|t| serde_json::from_str::<serde_json::Value>(t).unwrap())
        .find(|v| v["type"] == "speaker" && v["state"] != "verifying")
        .expect("an observe state frame");
    assert_eq!(speaker["state"], "unavailable");
    assert_eq!(actor.phase(), SessionPhase::Ready);
}

/// Ticket 15: one manual Required turn with the identity matching the locked candidate commits the
/// transcript and locks the identity.
#[tokio::test]
async fn required_turn_verifies_and_commits() {
    let (observe, _lease) = observe_with_policy(false, SpeakerPolicyMode::Required).await;
    let (control, mut messages) = mpsc::channel(8);
    let (audio, _) = mpsc::channel(1);
    let providers = providers();
    let mut actor = SessionActor::new("session".into(), control, audio, CAPTURE_FRAMES, providers)
        .unwrap()
        .with_speaker_observe(Some(observe), true);

    actor.on_client_message(ClientMessage::listen(ListenCommand::Start {
        mode: ListenMode::Manual,
    }));
    push_uplink(&mut actor);
    actor.on_client_message(ClientMessage::listen(ListenCommand::Stop));
    let messages = drain(&mut actor, &mut messages, true).await;

    // The transcript is only committed after a passing speaker result for the same turn.
    assert!(messages.iter().any(is_stt_frame));
    let speaker = messages
        .iter()
        .filter_map(|m| m.as_text())
        .map(|t| serde_json::from_str::<serde_json::Value>(t).unwrap())
        .find(|v| v["type"] == "speaker" && v["state"] != "verifying")
        .expect("a required state frame");
    assert_eq!(speaker["state"], "verified");
    assert_eq!(actor.phase(), SessionPhase::Ready);
}

/// Ticket 15: a Required session refuses typed Detect without committing it.
#[tokio::test]
async fn required_rejects_detect_without_audio() {
    let (observe, _lease) = observe_with_policy(false, SpeakerPolicyMode::Required).await;
    let (control, mut messages) = mpsc::channel(8);
    let (audio, _) = mpsc::channel(1);
    let providers = providers();
    let mut actor = SessionActor::new("session".into(), control, audio, CAPTURE_FRAMES, providers)
        .unwrap()
        .with_speaker_observe(Some(observe), true);

    actor.on_client_message(ClientMessage::listen(ListenCommand::Start {
        mode: ListenMode::Manual,
    }));
    actor.on_client_message(ClientMessage::listen(ListenCommand::Detect {
        text: "xin chào".into(),
    }));

    let mut collected = Vec::new();
    for _ in 0..200 {
        actor.pump_workers();
        while let Ok(message) = messages.try_recv() {
            collected.push(message);
        }
        if collected
            .iter()
            .filter_map(|m| m.as_text())
            .any(|t| t.contains("\"denied\""))
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    assert!(
        collected
            .iter()
            .filter_map(|m| m.as_text())
            .any(|t| t.contains("\"denied\"")),
        "expected a denied state frame"
    );
    // Detect text never reached the transcript.
    assert!(!collected.iter().any(is_stt_frame));
}

/// Ticket 15: an `off` session is unaffected — Detect still commits.
#[tokio::test]
async fn off_session_still_accepts_detect() {
    let (control, mut messages) = mpsc::channel(8);
    let (audio, _) = mpsc::channel(1);
    let providers = providers();
    let mut actor =
        SessionActor::new("session".into(), control, audio, CAPTURE_FRAMES, providers).unwrap();

    actor.on_client_message(ClientMessage::listen(ListenCommand::Start {
        mode: ListenMode::Manual,
    }));
    actor.on_client_message(ClientMessage::listen(ListenCommand::Detect {
        text: "xin chào".into(),
    }));
    let messages = drain(&mut actor, &mut messages, false).await;
    assert!(messages.iter().any(is_stt_frame));
    assert_eq!(actor.phase(), SessionPhase::Ready);
}
