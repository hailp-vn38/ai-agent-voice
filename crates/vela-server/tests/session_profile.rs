//! Effective Session Profile integration tests.
//!
//! Verifies:
//! - Admission & Resolution: server defaults fallback, fail-closed handling, immutability, and concurrency.
//! - Dynamic Template Switching: runtime swap at turn boundaries, snapshot isolation, interrupt handling, and audio frame retention.
//! - Managed Provider Runtimes: dynamic materialization, REST API CRUD, prepare gate, and admission flood control.
//! - Direct Speech: boundary commit before admin speech injection.

#[path = "session_profile/harness.rs"]
mod harness;

#[path = "session_profile/admission.rs"]
mod admission;

#[path = "session_profile/switch.rs"]
mod switch;

#[path = "session_profile/provider_runtime.rs"]
mod provider_runtime;

#[path = "session_profile/direct_speech.rs"]
mod direct_speech;
