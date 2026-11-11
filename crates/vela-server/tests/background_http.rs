//! Background Task & HTTP Admin integration tests.
//!
//! Verifies:
//! - Foundation: authenticated endpoints, fail-closed handling without executor, runtime health.
//! - Settings & Skills: background settings CRUD, revision concurrency, skills discovery & execution.
//! - Subagents & Jobs: subagent registration, job scheduling, async dispatch, and cancellation.

#[path = "support/admin_web_token.rs"]
mod admin_web_token_support;

#[path = "background_http/harness.rs"]
mod harness;

pub use harness::*;

#[path = "background_http/foundation.rs"]
mod foundation;

#[path = "background_http/settings_skills.rs"]
mod settings_skills;

#[path = "background_http/subagents_jobs.rs"]
mod subagents_jobs;
