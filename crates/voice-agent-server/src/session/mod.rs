mod actor;
mod event;
mod generation_gate;
mod profile;
mod prompt;
mod runtime_profile;
mod speaker_observe;
mod speech_output;
mod state;
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
pub use speaker_observe::{
    ObserveCandidate, ObserveDiagnostic, ObserveIdentity, ObservePlan, ObserveScore,
    SpeakerObserve, SpeakerPolicyMode, SpeakerStatus, resolve_observe_plan,
};
pub use state::SessionPhase;
pub use turn::{ActiveTurnLimiter, TurnId};

pub mod pilot;
