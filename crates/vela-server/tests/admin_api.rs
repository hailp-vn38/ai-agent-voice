//! Admin API integration tests.
//!
//! Verifies:
//! - Diagnostics: VAD, ASR, LLM, and TTS runtime and draft testing contracts.
//! - Providers & Templates: provider registration, mutation validation, template overrides, and read models.
//! - Credentials & MCP: encrypted secret storage, redaction, and external MCP configurations.
//! - Reminders: scheduling, replay, concurrency, quotas, and mutations.

#[path = "admin_api/harness.rs"]
mod harness;

pub use harness::*;

#[path = "admin_api/diagnostics.rs"]
mod diagnostics;

#[path = "admin_api/providers_templates.rs"]
mod providers_templates;

#[path = "admin_api/credentials_mcp.rs"]
mod credentials_mcp;

#[path = "admin_api/reminders.rs"]
mod reminders;
