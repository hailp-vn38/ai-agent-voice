//! Config Provider Bindings & Diagnostics integration tests.
//!
//! Verifies:
//! - Catalog & Bindings: configuration and database provider merging, template bindings mutations and audits.
//! - Websocket Admission: mixed provider source resolution, session lifetime isolation, and dynamic eviction.
//! - Diagnostics: diagnostic acquisition for cold deployment providers, bounds, timeouts, and quarantine.

#[path = "support/admin_web.rs"]
mod admin_web;

#[path = "config_provider_bindings/harness.rs"]
mod harness;

pub use harness::*;

#[path = "config_provider_bindings/catalog_bindings.rs"]
mod catalog_bindings;

#[path = "config_provider_bindings/websocket_admission.rs"]
mod websocket_admission;

#[path = "config_provider_bindings/diagnostics.rs"]
mod diagnostics;
