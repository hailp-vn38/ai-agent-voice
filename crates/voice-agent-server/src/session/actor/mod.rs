use crate::session::{
    ActiveTurnLimiter, GenerationGate, SessionPhase,
    event::SessionEvent,
    speech_output::{SpeechOutput, SpeechOutputEvent},
    turn::DialogueHistory,
};
use crate::{
    audio::{
        CaptureOutcome, DecodeOutcome, ManualCapture, PcmF32Mono, UplinkOpusDecoder, VadBoundary,
        VadSegmenter, VadSegmenterConfig,
    },
    protocol::{ClientMessage, ListenCommand, ListenMode},
    providers::{ProviderSet, llm::UnavailableLlm, tts::UnavailableTts},
    workers::{
        AsrCommand, AsrStreamLease, AsrWorkerEvent, AsrWorkerRuntime, LlmRuntime, LlmRuntimeEvent,
        TtsWorkerRuntime, VadCaptureCycleId, VadCommand, VadWorkerEvent, VadWorkerLease,
        VadWorkerRuntime, WorkerIdentity, WorkerRuntimeConfig,
    },
};
use std::collections::HashSet;

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
    has_active_turn_permit: bool,
    generation: u64,
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
    control_tx: mpsc::Sender<OutboundMessage>,
    urgent_tx: mpsc::Sender<OutboundMessage>,
    shutdown_tx: watch::Sender<bool>,
    audio_tx: mpsc::Sender<OutboundMessage>,
    generation_gate: std::sync::Arc<GenerationGate>,
    /// A packet removed from SpeechOutput but not yet admitted by the bounded writer queue.
    /// It must be retried before polling another packet: dropping it creates audible gaps.
    pending_audio: Option<OutboundMessage>,
    client_aec_asserted: bool,
    barge_in_enabled: bool,
    trust_client_aec_feature: bool,
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
    generation: u64,
    cancellation: CancellationToken,
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
    TurnText { generation: u64, text: String },
    Binary { generation: u64, packet: Vec<u8> },
    Close(u16),
}

impl OutboundMessage {
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(text) | Self::TurnText { text, .. } => Some(text),
            Self::Binary { .. } | Self::Close(_) => None,
        }
    }
}

mod construct;
mod delivery;
mod ingress;
mod lifecycle;
mod listening;
fn normalize_detect_text(input: String) -> Option<String> {
    let text = input.trim();
    if text.is_empty()
        || text.chars().take(MAX_DETECT_TEXT_SCALARS + 1).count() > MAX_DETECT_TEXT_SCALARS
    {
        return None;
    }
    Some(text.to_owned())
}
