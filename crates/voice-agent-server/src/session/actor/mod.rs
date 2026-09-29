use crate::session::{
    ActiveTemplateProfile, ActiveTurnLimiter, GenerationGate, SessionPhase, TemplateSwitchCatalog,
    TurnId,
    event::SessionEvent,
    speech_output::{SpeechOutput, SpeechOutputEvent},
    turn::{ActiveTurnPermit, DialogueHistory},
};
use crate::{
    audio::{
        CaptureOutcome, DecodeOutcome, ManualCapture, PcmF32Mono, UplinkOpusDecoder, VadBoundary,
        VadSegmenter, VadSegmenterConfig,
    },
    protocol::{ClientMessage, ListenCommand, ListenMode},
    providers::{
        ProviderSet,
        llm::UnavailableLlm,
        llm::{ChatMessage, ToolCall},
        tts::UnavailableTts,
    },
    tools::device_mcp::{DiscoveredTool, LlmVisibleTool, McpRequestId, VisionCapability},
    workers::{
        AsrCommand, AsrStreamLease, AsrWorkerEvent, AsrWorkerRuntime, LlmRuntime, LlmRuntimeEvent,
        TtsWorkerRuntime, VadCaptureCycleId, VadCommand, VadWorkerEvent, VadWorkerLease,
        VadWorkerRuntime, WorkerIdentity, WorkerRuntimeConfig,
    },
};
use std::{
    collections::{HashMap, HashSet},
    time::Instant,
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

/// The only mutable owner of an accepted Voice Session's phase.
pub struct SessionActor {
    session_id: String,
    phase: SessionPhase,
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
    tts_runtime: std::sync::Arc<TtsWorkerRuntime>,
    speech_output: SpeechOutput,
    tts_started: bool,
    pending_delivery: Option<PendingDelivery>,
    pending_actions: PendingSessionActions,
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
    tool_depth: usize,
    max_tool_depth: usize,
    max_tool_result_chars: usize,
    /// The one configuration snapshot this session runs with.  Admission installs it; a successful
    /// switch replaces it whole at a turn boundary.
    profile: ActiveTemplateProfile,
    /// Admission-time switch candidates.  Never re-read, never grown, never pruned.
    switch_catalog: TemplateSwitchCatalog,
    /// Delivery settings are retained so a switch can rebuild SpeechOutput on the candidate's
    /// already-loaded TTS runtime without re-deriving pacing behavior.
    speech_output_config: crate::config::SpeechOutputConfig,
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
}

#[derive(Clone, Copy, Debug)]
enum TurnFailure {
    LlmRequestTooLarge,
    ToolDepthExceeded,
}

impl TurnFailure {
    fn code(self) -> &'static str {
        match self {
            Self::LlmRequestTooLarge => "llm_request_too_large",
            Self::ToolDepthExceeded => "tool_depth_exceeded",
        }
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
struct ToolBatchState {
    generation: u64,
    calls: Vec<ToolCall>,
    completed_calls: Vec<ToolCall>,
    next: usize,
    results: Vec<ChatMessage>,
    direct_response: Option<String>,
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
}

impl OutboundMessage {
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(text)
            | Self::TurnText { text, .. }
            | Self::BeginTurn { text, .. }
            | Self::FinishTurn { text, .. }
            | Self::AbortTurn { text, .. } => Some(text),
            Self::Binary { .. } | Self::Close(_) => None,
        }
    }
}

mod construct;
mod delivery;
mod ingress;
mod lifecycle;
mod listening;
mod mcp;
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
