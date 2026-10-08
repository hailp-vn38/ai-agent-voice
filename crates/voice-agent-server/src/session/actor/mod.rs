use crate::session::{
    ActiveTemplateProfile, ActiveTurnLimiter, GateDecision, GateReject, GenerationGate,
    ObserveDiagnostic, ProfileSource, SessionDeviceTools, SessionPhase, SpeakerGate,
    SpeakerObserve, SpeakerSwitchGuard, TemplateSwitchCatalog, TurnId,
    event::SessionEvent,
    speech_output::{SpeechOutput, SpeechOutputEvent},
    turn::{ActiveTurnPermit, DialogueHistory},
};
use crate::{
    audio::{
        CaptureOutcome, DecodeOutcome, ManualCapture, PcmF32Mono, UplinkOpusDecoder, VadBoundary,
        VadSegmenter, VadSegmenterConfig,
    },
    database::history::{HistoryRole, TranscriptCapture},
    lifecycle::AdmissionGate,
    protocol::{ClientMessage, ListenCommand, ListenMode},
    providers::{
        ProviderSet,
        llm::UnavailableLlm,
        llm::{ChatMessage, ToolCall},
        tts::UnavailableTts,
    },
    tools::{
        device_mcp::{DiscoveredTool, LlmVisibleTool, McpRequestId, VisionCapability},
        external_mcp::{ExternalMcpError, ExternalToolOutcome, SessionExternalMcp},
        round::{ToolRoundFailure, ToolRoundLimits},
    },
    workers::{
        AsrCommand, AsrStreamLease, AsrWorkerEvent, AsrWorkerRuntime, LlmRuntime, LlmRuntimeEvent,
        TtsWorkerRuntime, VadCaptureCycleId, VadCommand, VadWorkerEvent, VadWorkerLease,
        VadWorkerRuntime, WorkerIdentity, WorkerRuntimeConfig,
    },
};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    time::{Duration, Instant},
};

use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

mod retention;

use retention::{AutoPcmRetention, auto_retention_capacity};

const MAX_DETECT_TEXT_SCALARS: usize = 4_096;

/// Per-session LLM event buffer.  A switch re-registers this session on the candidate's runtime,
/// so the capacity must not drift between the initial registration and a switched one.
const LLM_EVENT_CAPACITY: usize = 64;

/// The Tool-round Executor's own mailbox for External Tool Call completions.
///
/// The executor is strictly sequential, so at most one completion is ever outstanding and this
/// never queues.  The bound is two only so a completion that arrives between the executor applying
/// it and starting the next call still has somewhere to land.
const EXTERNAL_CALL_CAPACITY: usize = 2;
const DEVICE_TOOLS_CAPACITY: usize = 2;

/// The only mutable owner of an accepted Voice Session's phase.
pub struct SessionActor {
    session_id: String,
    phase: SessionPhase,
    pilot_admission: crate::session::pilot::PilotAdmission,
    pipeline_permit: Option<crate::session::pilot::PilotPermit>,
    pipeline_status: bool,
    pipeline_request: std::sync::Arc<std::sync::atomic::AtomicU64>,
    pipeline_writer_pending: HashSet<TurnId>,
    pipeline_writer_terminal: Option<watch::Receiver<bool>>,
    accepted_binary_frames: u64,
    uplink_decoder: UplinkOpusDecoder,
    manual_capture: ManualCapture,
    asr_runtime: std::sync::Arc<AsrWorkerRuntime>,
    asr_events: mpsc::Receiver<AsrWorkerEvent>,
    asr_stream: Option<(AsrStreamLease, WorkerIdentity)>,
    asr_cleanup_pending: HashSet<WorkerIdentity>,
    vad_runtime: std::sync::Arc<VadWorkerRuntime>,
    vad_events: mpsc::Receiver<VadWorkerEvent>,
    vad_session: Option<(VadWorkerLease, WorkerIdentity)>,
    vad_cycle: Option<VadCaptureCycleId>,
    pending_vad_cycle: Option<VadCaptureCycleId>,
    next_vad_cycle: u64,
    listening_mode: Option<ListenMode>,
    listen_arm_pending: bool,
    auto_speech_active: bool,
    auto_reset_pending: bool,
    vad_segmenter: VadSegmenter,
    auto_retention: AutoPcmRetention,
    pre_roll_samples: u64,
    active_turn_limiter: std::sync::Arc<ActiveTurnLimiter>,
    generation: u64,
    next_turn_id: u64,
    next_operation_id: u64,
    turn: Option<TurnContext>,
    dialogue_history: DialogueHistory,
    llm_runtime: std::sync::Arc<LlmRuntime>,
    llm_events: mpsc::Receiver<LlmRuntimeEvent>,
    llm_operation: Option<WorkerIdentity>,
    pending_llm_delta: Option<(String, usize)>,
    llm_finish_pending: bool,
    generated_response: String,
    /// Whether `generated_response` holds the model's own text rather than a tool's result.
    ///
    /// A round that starts or continues an LLM operation clears it, so the only way it can be set
    /// is the seam where a tool result is handed straight to speech.  It is what keeps a delivered
    /// tool result out of the Persistent Transcript.
    generated_by_model: bool,
    tts_runtime: std::sync::Arc<TtsWorkerRuntime>,
    speech_output: SpeechOutput,
    tts_started: bool,
    pending_delivery: Option<PendingDelivery>,
    pending_actions: PendingSessionActions,
    template_prepare: Option<TemplatePreparation>,
    managed_switch_boundary: Option<crate::session::PreparedTemplateProfile>,
    managed_switch_started: Option<Instant>,
    deferred_switch_ingress: VecDeque<SessionEvent>,
    switch_ingress_capacity: usize,
    switch_max_frame_bytes: usize,
    control_tx: mpsc::Sender<OutboundMessage>,
    urgent_tx: mpsc::Sender<OutboundMessage>,
    shutdown_tx: watch::Sender<bool>,
    audio_tx: mpsc::Sender<OutboundMessage>,
    generation_gate: std::sync::Arc<GenerationGate>,
    writer_events: Option<mpsc::Receiver<WriterEvent>>,
    /// A packet removed from SpeechOutput but not yet admitted by the bounded writer queue.
    /// It must be retried before polling another packet: dropping it creates audible gaps.
    pending_audio: Option<OutboundMessage>,
    client_aec_asserted: bool,
    barge_in_enabled: bool,
    trust_client_aec_feature: bool,
    mcp: DeviceMcpState,
    tool_batch: Option<ToolBatchState>,
    llm_messages: Vec<ChatMessage>,
    llm_round: Option<LlmRoundBuffer>,
    /// The shared Tool-round Executor's per-turn state: the caps this deployment validated, how
    /// many tool rounds this turn has continued past its first, and the Tool Execution Budget that
    /// started at its first ToolCall.
    tool_rounds: ToolRoundState,
    /// Where an External Tool Call this session started reports back.
    external_calls_tx: mpsc::Sender<ExternalCallCompletion>,
    external_calls: mpsc::Receiver<ExternalCallCompletion>,
    /// Device discovery completes out of band so its observation write cannot block the actor.
    device_tools_tx: mpsc::Sender<DeviceToolsCompletion>,
    device_tools_rx: mpsc::Receiver<DeviceToolsCompletion>,
    max_tool_result_chars: usize,
    /// The one configuration snapshot this session runs with.  Admission installs it; a successful
    /// switch replaces it whole at a turn boundary.
    profile: ActiveTemplateProfile,
    /// Admission-time switch candidates.  Never re-read, never grown, never pruned.
    switch_catalog: TemplateSwitchCatalog,
    /// The External MCP tools admission resolved, held as immutable handles.
    ///
    /// The actor owns the snapshot, never the credential behind it: each `ResolvedExternalMcp`
    /// keeps its `SecretValue` inside the client handle, so nothing here can be logged or printed
    /// and no hot path can re-resolve anything.  Admitting, calling and advertising these tools
    /// belongs to the shared Tool-round Executor, which this snapshot exists to feed.
    external_mcp: SessionExternalMcp,
    /// The Device tool review this Voice Session was admitted under, if any.  A session admitted
    /// without one keeps the legacy Device allowlist behavior; one admitted with a participating
    /// Agent only calls what that Agent approved for this Device incarnation.
    device_tools: SessionDeviceTools,
    /// Delivery settings are retained so a switch can rebuild SpeechOutput on the candidate's
    /// already-loaded TTS runtime without re-deriving pacing behavior.
    speech_output_config: crate::config::SpeechOutputConfig,
    /// The optional Persistent Transcript this Voice Session was bound to at admission.  `None` is
    /// the normal case: capture is opt-in, and it is a one-way hand-off that no turn can fail on.
    transcript: Option<TranscriptCapture>,
    /// Ticket 10 Observe: the exact selected speaker extractor, its lease and the active Template's
    /// plan. `None` when the Agent policy is `off`, so the core path retains no utterance PCM and
    /// runs no speaker inference.
    speaker_observe: Option<std::sync::Arc<SpeakerObserve>>,
    /// Whether the client opted into bounded `speaker` status frames. Separate from `pipeline_status`.
    speaker_status: bool,
    /// Bounded utterance PCM, retained only while Observe is installed. Cleared at every terminal
    /// boundary and at every new utterance start; never grows past `OBSERVE_MAX_SAMPLES`.
    observe_pcm: Vec<f32>,
    /// At most one Observe extraction in flight. A boundary that arrives while one runs is dropped,
    /// never queued, so Observe can never apply backpressure to the core path. Shared with the
    /// detached scoring task, which clears it on completion.
    observe_in_flight: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Ticket 15 Required: the per-WebSocket identity lock and consecutive-denial counter. `None`
    /// unless the Agent policy is `required`; then no turn reaches history or the LLM without a
    /// fresh speaker pass for that same turn.
    speaker_gate: Option<SpeakerGate>,
    /// Ticket 17: admission-time authority to switch the locked Speaker onto another Template.
    /// `None` when the Agent policy is `off`, so a speaker-free session keeps membership as the
    /// whole switch rule.
    speaker_switch: Option<std::sync::Arc<SpeakerSwitchGuard>>,
    /// The ASR final of the active Required turn, held until the speaker operation resolves.
    required_text: Option<String>,
    /// The speaker diagnostic of the active Required turn, held until the ASR final resolves.
    required_diagnostic: Option<ObserveDiagnostic>,
    /// Where the detached speaker scoring task reports the diagnostic for the Required gate.
    gate_tx: mpsc::UnboundedSender<ObserveDiagnostic>,
    gate_rx: mpsc::UnboundedReceiver<ObserveDiagnostic>,
    /// The application admission gate.  A Voice Session does not own it and cannot reopen it: it
    /// only asks, so a Tool-round Executor cannot start new work after the application has stopped
    /// accepting it, whether or not this session ever observed the shutdown signal.
    admission_gate: std::sync::Arc<AdmissionGate>,
    /// Installed only by a test harness; see [`WriterOutcomeProbe`].
    writer_probe: Option<std::sync::Arc<dyn WriterOutcomeProbe>>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BargeInPolicy {
    pub enabled: bool,
    pub trust_client_aec_feature: bool,
}

impl BargeInPolicy {
    pub fn allows(&self, client_aec_asserted: bool, mode: Option<ListenMode>) -> bool {
        self.enabled
            && self.trust_client_aec_feature
            && client_aec_asserted
            && matches!(mode, Some(ListenMode::Auto | ListenMode::Realtime))
    }
}

/// Cancellation ownership for one Conversational Turn.
///
/// The token is never reused: interrupting a turn cancels this instance, and the
/// next accepted user utterance receives a fresh context.
struct TurnContext {
    turn_id: TurnId,
    generation: u64,
    cancellation: CancellationToken,
    permit: ActiveTurnPermit,
}

struct PendingDelivery {
    turn_id: TurnId,
    assistant_text: String,
    /// Whether this text is the model's own Delivered Assistant Response and therefore belongs in
    /// the Persistent Transcript.
    ///
    /// A Device MCP tool answered directly, and the session-local built-in actions speak their own
    /// text; both put a tool's result where the model's text goes.  Dialogue History still commits
    /// it exactly as before — that is RAM conversational state and not the archive's business —
    /// but a tool result is never archived.
    archives_as_assistant: bool,
}

/// Session actions the Voice Session itself requested that may only take effect once the current
/// normal Conversational Turn has closed.  Keeping them apart means an exit and a Template switch
/// armed in the same turn neither overwrites nor cancels the other.
#[derive(Default)]
struct PendingSessionActions {
    close_after_turn: Option<TurnId>,
    switch_template_after_turn: Option<PendingTemplateSwitch>,
}

impl PendingSessionActions {
    /// Takes whatever this turn armed and leaves anything another turn armed in place.
    fn take_for_turn(&mut self, turn_id: TurnId) -> Self {
        Self {
            close_after_turn: self
                .close_after_turn
                .take()
                .filter(|pending| *pending == turn_id),
            switch_template_after_turn: self
                .switch_template_after_turn
                .take()
                .filter(|pending| pending.turn_id == turn_id),
        }
    }

    fn is_empty(&self) -> bool {
        self.close_after_turn.is_none() && self.switch_template_after_turn.is_none()
    }
}

/// A Template this session admitted, scheduled to become active at the next turn boundary.
struct PendingTemplateSwitch {
    turn_id: TurnId,
    template_key: String,
    prepared: Option<crate::session::PreparedTemplateProfile>,
}
struct TemplatePreparation {
    turn_id: TurnId,
    generation: u64,
    profile_revision: u64,
    call: ToolCall,
    completion: tokio::sync::oneshot::Receiver<
        Result<
            crate::session::PreparedTemplateProfile,
            crate::services::provider_runtime::RuntimeError,
        >,
    >,
}

#[derive(Clone, Copy, Debug)]
enum TurnFailure {
    LlmRequestTooLarge,
    Tool(ToolRoundFailure),
}

impl TurnFailure {
    fn code(self) -> &'static str {
        match self {
            Self::LlmRequestTooLarge => "llm_request_too_large",
            Self::Tool(failure) => failure.code(),
        }
    }
}

/// The Tool-round Executor's per-turn bookkeeping.
///
/// The caps live here rather than being re-read from configuration, so a turn asks one value "what
/// am I allowed to do" instead of consulting the deployment on every call — and because a session
/// that holds one of these is a deployment that either started or failed to start.
#[derive(Clone, Copy, Debug)]
struct ToolRoundState {
    limits: ToolRoundLimits,
    /// Tool rounds this turn has continued past its first.  The cap is checked before the next
    /// round's request is sent, so a turn never starts a round it cannot finish.
    rounds: usize,
    /// When this turn's first ToolCall started, or `None` while it has called no tool.  The Tool
    /// Execution Budget is spent from here, so a turn that never calls a tool never has one.
    budget_started: Option<Instant>,
}

impl ToolRoundState {
    fn new(limits: ToolRoundLimits) -> Self {
        Self {
            limits,
            rounds: 0,
            budget_started: None,
        }
    }

    /// Resets everything a turn does not carry into the next one.  The budget deliberately does not
    /// survive: a Tool Execution Budget belongs to one Conversational Turn, and a new turn starts
    /// with the full budget and no round counted against it.
    fn begin_turn(&mut self) {
        self.rounds = 0;
        self.budget_started = None;
    }

    /// The bound for the call about to start, which is also where this turn's budget starts.
    ///
    /// `None` means the turn has nothing left to spend, which is a turn-level failure rather than
    /// something to send: a call started with nothing behind it could produce a side effect whose
    /// result could never be reported.
    fn budget_for_next_call(&mut self) -> Option<Duration> {
        self.budget_started.get_or_insert_with(Instant::now);
        self.remaining()
    }

    /// What is left of the turn's budget, or `None` once it is spent.
    ///
    /// Pure: reading the budget never starts it, so a turn that has called no tool has none to
    /// report and is not judged against it.
    fn remaining(&self) -> Option<Duration> {
        let started = self.budget_started?;
        let left = self
            .limits
            .execution_budget
            .saturating_sub(started.elapsed());
        (!left.is_zero()).then_some(left)
    }
}

#[derive(Default)]
struct DeviceMcpState {
    enabled: bool,
    ready: bool,
    failed: bool,
    next_request_id: u64,
    allowed_tools: HashSet<String>,
    result_delivery: crate::config::McpResultDelivery,
    tool_delivery: HashMap<String, crate::config::McpResultDelivery>,
    call_timeout: std::time::Duration,
    discovery_timeout: std::time::Duration,
    discovered: Vec<DiscoveredTool>,
    visible: Vec<LlmVisibleTool>,
    pending: HashMap<McpRequestId, PendingMcpRequest>,
    vision: Option<VisionCapability>,
}

struct PendingMcpRequest {
    generation: Option<u64>,
    deadline: Instant,
    kind: PendingMcpKind,
}
enum PendingMcpKind {
    Initialize,
    ToolsList,
    ToolCall { call: ToolCall },
}
/// One tool round in flight.
///
/// `calls` is the whole round exactly as the model returned it, and `completed_calls` / `results`
/// are the prefix that has terminalized.  The two stay index-aligned, and the round is only handed
/// to history and to the continuation when they are the same length — which is why a call may only
/// be recorded by [`SessionActor::record_tool_call`], the one place both are pushed.
struct ToolBatchState {
    generation: u64,
    calls: Vec<ToolCall>,
    completed_calls: Vec<ToolCall>,
    next: usize,
    results: Vec<ChatMessage>,
    direct_response: Option<String>,
    /// The External Tool Call this round is waiting for, if any.
    ///
    /// The executor is strictly sequential, so there is never more than one: a second call cannot
    /// start until this one has produced its terminal result or has been dropped by cancellation.
    in_flight: Option<InFlightExternalCall>,
    /// Resolved once, when the round began, so nothing re-derives it while the round runs.
    delivery: crate::config::McpResultDelivery,
}

/// The one External Tool Call a round is waiting for.
///
/// Its turn and generation are recorded so a completion arriving after either moved on is
/// recognisable as late rather than as the next round's work.
struct InFlightExternalCall {
    /// The LLM ToolCall this call will produce a result for.
    call: ToolCall,
    turn_id: TurnId,
    generation: u64,
}

/// What one External Tool Call left behind.
///
/// Identity rides along instead of being looked up: recognising a completion as late is the whole
/// purpose of this type, and a lookup could not tell a late response from a fresh one.
struct ExternalCallCompletion {
    turn_id: TurnId,
    generation: u64,
    /// The `tool_call_id` this completion answers, matched against the in-flight call.
    call_id: String,
    /// The server that answered, so a discarded response is still counted against the right one.
    /// Bounded to `[a-z0-9_]` at admission, which is what makes it a safe metric label.
    server_key: String,
    outcome: Result<ExternalToolOutcome, ExternalMcpError>,
}

/// One completed Device `tools/list` walk, handed back with the admitted snapshot it was recorded
/// under so the actor can derive the visible catalog without touching the database again.
struct DeviceToolsCompletion {
    discovered: Vec<DiscoveredTool>,
    contracts: std::collections::HashMap<String, String>,
    participating: bool,
}

#[derive(Default)]
struct LlmRoundBuffer {
    prose: String,
    calls: Vec<ToolCall>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WriterTurnOutcome {
    Normal,
    Aborted {
        start_was_sent: bool,
        stop_was_sent: bool,
    },
}

/// Test-only observation point at the writer's terminal-outcome boundary.
///
/// A Voice Protocol Client learns a turn has closed from `tts:stop`, which the writer sends
/// immediately *before* it reports the turn's terminal outcome to the actor. Nothing visible to
/// the client synchronizes those two steps, so a black-box client can only ever probe the ordering
/// probabilistically. This probe lets a test harness hold the writer at the exact boundary and
/// learn the instant the outcome has entered the actor's mailbox, which makes that ordering
/// testable deterministically.
///
/// Production installs no probe, so the writer's behaviour and the actor's periodic drain are
/// byte-for-byte what they were.
#[async_trait::async_trait]
pub trait WriterOutcomeProbe: Send + Sync {
    /// Awaited by the writer immediately before it reports a turn's terminal outcome.
    async fn before_terminal_outcome(&self, _turn_id: TurnId, _outcome: WriterTurnOutcome) {}

    /// Awaited by the writer immediately after that outcome entered the actor's mailbox.
    async fn after_terminal_outcome_reported(&self, _turn_id: TurnId, _outcome: WriterTurnOutcome) {
    }

    /// True while a harness deliberately withholds already-reported outcomes from the actor's
    /// periodic drain, so the only remaining path that can apply one is client ingress.
    fn holds_writer_outcomes(&self) -> bool {
        false
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WriterEvent {
    TurnClosed {
        turn_id: TurnId,
        outcome: WriterTurnOutcome,
    },
    Failed {
        turn_id: Option<TurnId>,
    },
}

/// Application-owned runtimes shared by every voice session.
pub struct SessionRuntimes {
    pub asr: std::sync::Arc<AsrWorkerRuntime>,
    pub vad: std::sync::Arc<VadWorkerRuntime>,
    pub llm: std::sync::Arc<LlmRuntime>,
    pub tts: std::sync::Arc<TtsWorkerRuntime>,
    pub active_turn_limiter: std::sync::Arc<ActiveTurnLimiter>,
    pub vad_segmenter_config: VadSegmenterConfig,
    pub pre_roll_samples: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OutboundMessage {
    Text(String),
    PipelineStatus {
        request: u64,
        text: String,
    },
    /// Ticket 10: a bounded Observe state frame. Carries only the bounded state, never identity
    /// or score, and is sent with the same fire-and-forget backpressure as `PipelineStatus`.
    SpeakerStatus {
        text: String,
    },
    TurnText {
        generation: u64,
        turn_id: TurnId,
        text: String,
    },
    BeginTurn {
        generation: u64,
        turn_id: TurnId,
        text: String,
    },
    FinishTurn {
        turn_id: TurnId,
        text: String,
    },
    AbortTurn {
        turn_id: TurnId,
        text: String,
    },
    Binary {
        generation: u64,
        turn_id: TurnId,
        packet: Vec<u8>,
    },
    Close(u16),
    /// Ticket 15: a close carrying a bounded, content-free reason (e.g. speaker refusal).
    CloseWithReason {
        code: u16,
        reason: String,
    },
}

impl OutboundMessage {
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(text)
            | Self::PipelineStatus { text, .. }
            | Self::SpeakerStatus { text }
            | Self::TurnText { text, .. }
            | Self::BeginTurn { text, .. }
            | Self::FinishTurn { text, .. }
            | Self::AbortTurn { text, .. } => Some(text),
            Self::Binary { .. } | Self::Close(_) | Self::CloseWithReason { .. } => None,
        }
    }
}

mod construct;
mod delivery;
mod ingress;
mod lifecycle;
mod listening;
mod mcp;
mod observe;
mod pilot;
mod speaker;
mod tools;
fn normalize_detect_text(input: String) -> Option<String> {
    let text = input.trim();
    if text.is_empty()
        || text.chars().take(MAX_DETECT_TEXT_SCALARS + 1).count() > MAX_DETECT_TEXT_SCALARS
    {
        return None;
    }
    Some(text.to_owned())
}

impl SessionActor {
    /// Allocates a semantic identity only after Active Turn admission succeeds.
    pub(super) fn begin_active_turn(&mut self) -> Option<TurnId> {
        let permit = ActiveTurnLimiter::try_acquire_permit(&self.active_turn_limiter)?;
        let raw = self.next_turn_id;
        let Some(next) = raw.checked_add(1) else {
            drop(permit);
            self.fail_closed();
            return None;
        };
        let Some(turn_id) = TurnId::new(raw) else {
            drop(permit);
            self.fail_closed();
            return None;
        };
        self.next_turn_id = next;
        self.turn = Some(TurnContext {
            turn_id,
            generation: self.generation,
            cancellation: CancellationToken::new(),
            permit,
        });
        Some(turn_id)
    }

    pub(super) fn next_worker_identity(&mut self) -> Option<WorkerIdentity> {
        let operation_id = self.next_operation_id;
        self.next_operation_id = operation_id.checked_add(1)?;
        Some(WorkerIdentity::new(
            self.session_id.clone(),
            self.generation,
            operation_id,
        ))
    }

    pub(super) fn current_turn_id(&self) -> Option<TurnId> {
        self.turn.as_ref().map(|turn| turn.turn_id)
    }

    /// The Template this Voice Session is running, or `None` while it is on server defaults.
    ///
    /// Read at the moment a record is produced, never afterwards: a switch that is armed but not
    /// yet applied has not changed what the turn was run on, so the archive keeps the Template the
    /// turn actually used.
    fn active_template_id(&self) -> Option<i64> {
        match &self.profile.source {
            ProfileSource::Template { template_id, .. } => Some(*template_id),
            ProfileSource::ServerDefault => None,
        }
    }

    /// The one place a Voice Session produces a Persistent Transcript record.
    ///
    /// A session with no capture produces nothing at all, and producing nothing is the outcome when
    /// capture is off rather than a special case: there is no record to enqueue and no writer to
    /// call.  Both roles are archived exactly once, at the two seams where the text becomes final.
    fn record_transcript(&mut self, role: HistoryRole, text: &str, turn_id: TurnId) {
        let template_id = self.active_template_id();
        let turn_id = turn_id.get().to_string();
        if let Some(capture) = self.transcript.as_mut() {
            capture.record(role, template_id, &turn_id, text);
        }
    }

    /// Generation is a cancellation epoch, never an identity reused after overflow.
    pub(super) fn advance_generation(&mut self) -> bool {
        let Some(next) = self.generation.checked_add(1) else {
            self.phase = SessionPhase::Closed;
            let _ = self.urgent_tx.try_send(OutboundMessage::Close(1011));
            return false;
        };
        self.generation = next;
        true
    }
}
