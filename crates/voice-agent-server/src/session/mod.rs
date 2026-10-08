mod actor;
mod event;
mod generation_gate;
mod profile;
mod prompt;
mod runtime_profile;
mod speaker_gate;
mod speaker_observe;
mod speech_output;
mod state;
mod switch_authority;
mod turn;

pub use actor::{
    BargeInPolicy, OutboundMessage, SessionActor, SessionRuntimes, WriterEvent, WriterOutcomeProbe,
    WriterTurnOutcome,
};
pub use event::SessionEvent;
pub use generation_gate::GenerationGate;
pub use profile::{
    ActiveTemplateProfile, AdmittedSessionProfile, EffectiveSessionProfile,
    ManagedSessionProfileInput, ProfileSource, ProfileUnavailable, ResolvedTemplateProfile,
    SessionDeviceTools, TemplateSwitchCatalog, resolve_effective_session_profile,
    resolve_effective_session_profile_with_override, resolve_managed_session_profile,
};
pub use runtime_profile::{ConfiguredTemplateProfile, PreparedTemplateProfile};
pub use speaker_gate::{
    GateDecision, GateReject, SPEAKER_GATE_MARGIN, SPEAKER_GATE_MAX_MISMATCHES, SpeakerGate,
};
pub use speaker_observe::{
    ObserveCandidate, ObserveDiagnostic, ObserveIdentity, ObservePlan, ObserveResolution,
    ObserveScore, SpeakerObserve, SpeakerPolicyMode, SpeakerStatus, resolve_observe_plan,
    OBSERVE_VERIFY_THRESHOLD,
};
pub use state::SessionPhase;
pub use switch_authority::{SpeakerSwitchGuard, SwitchReject, SwitchSpeakerAuthority};
pub use turn::{ActiveTurnLimiter, TurnId};

pub mod pilot;
