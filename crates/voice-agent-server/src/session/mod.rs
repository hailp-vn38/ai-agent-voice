mod actor;
mod admission;
pub(crate) mod interaction;
pub(crate) mod speaker;
mod speech_output;

pub use actor::{
    BargeInPolicy, OutboundMessage, SessionActor, SessionRuntimes, WriterEvent, WriterOutcomeProbe,
    WriterTurnOutcome,
};
pub use admission::{
    ActiveTemplateProfile, AdmittedSessionProfile, ConfiguredTemplateProfile,
    EffectiveSessionProfile, ManagedSessionProfileInput, PilotAdmission, PreparedTemplateProfile,
    ProfileSource, ProfileUnavailable, ResolvedTemplateProfile, TemplateSwitchCatalog,
    resolve_effective_session_profile, resolve_effective_session_profile_with_override,
    resolve_managed_session_profile,
};
pub use interaction::ActiveTurnLimiter;
pub use interaction::{
    DirectSpeechError, DirectSpeechRequest, SpeechOrigin, validate_direct_speech,
};
pub use interaction::{GenerationGate, SessionEvent, SessionPhase, TurnId};
pub use speaker::{
    OBSERVE_VERIFY_THRESHOLD, ObserveCandidate, ObserveDiagnostic, ObserveIdentity, ObservePlan,
    ObserveResolution, ObserveScore, SpeakerContext, SpeakerObserve, SpeakerPolicyMode,
    SpeakerStatus, resolve_observe_plan, resolve_speaker_policy,
};

// Transitional module paths keep intra-crate callers stable while responsibilities live below.
pub use admission::pilot;
pub(crate) use admission::profile;
pub(crate) use interaction::prompt;
pub(crate) use speaker::observe as speaker_observe;
