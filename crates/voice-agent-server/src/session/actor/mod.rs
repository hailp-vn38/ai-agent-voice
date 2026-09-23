use crate::session::{
    ActiveTurnLimiter, GenerationGate, SessionPhase, TurnId,
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
    tools::device_mcp::{DiscoveredTool, LlmVisibleTool, McpRequestId},
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
    llm_messages: Vec<ChatMessage>,
    llm_round: Option<LlmRoundBuffer>,
    tool_depth: usize,
    max_tool_depth: usize,
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
    batch: Option<ToolBatchState>,
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
    next: usize,
    results: Vec<ChatMessage>,
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
