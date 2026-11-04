pub mod pilot;
pub(crate) mod profile;
mod runtime_profile;

pub use pilot::PilotAdmission;
pub use profile::{
    ActiveTemplateProfile, AdmittedSessionProfile, EffectiveSessionProfile,
    ManagedSessionProfileInput, ProfileSource, ProfileUnavailable, ResolvedTemplateProfile,
    TemplateSwitchCatalog, resolve_effective_session_profile,
    resolve_effective_session_profile_with_override, resolve_managed_session_profile,
};
pub use runtime_profile::{ConfiguredTemplateProfile, PreparedTemplateProfile};
