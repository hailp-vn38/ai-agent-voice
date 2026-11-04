//! Provider Materializer integration tests.
//!
//! Verifies:
//! - Materialization: single and remote provider materialization, unload lifecycle, configuration budgets.
//! - Identity: adapter-owned physical specifications, resource keys, and execution isolation.
//! - ZeroTTS: physical runtime sharing, voice aliasing, pool reservation, and replica alignment.
//! - Assets: asset preparation delegation, remote provider bypass, and startup default materialization.

#[path = "provider_materializer/harness.rs"]
mod harness;

pub use harness::*;

#[path = "provider_materializer/materialization.rs"]
mod materialization;

#[path = "provider_materializer/identity.rs"]
mod identity;

#[path = "provider_materializer/zerotts.rs"]
mod zerotts;

#[path = "provider_materializer/assets.rs"]
mod assets;
