//! Memory Admin API integration tests.
//!
//! Verifies:
//! - Routes & Guards: device parameters, disabled/unavailable states, schema validation, and wiki isolation.
//! - Facts & Profile: fact lifecycle (CRUD, search, pin, revision concurrency), discard tombstones, and profile.
//! - Devices & Vault: device memory partitioning, agent re-assignment, vault purging, and startup reconciliation.

#[path = "memory_admin/harness.rs"]
mod harness;

pub use harness::*;

#[path = "memory_admin/routes_guard.rs"]
mod routes_guard;

#[path = "memory_admin/facts_profile.rs"]
mod facts_profile;

#[path = "memory_admin/devices_vault.rs"]
mod devices_vault;
