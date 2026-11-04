//! The optional Persistent Transcript, proved at the two seams it is allowed to touch: a real
//! WebSocket Voice Session and the authenticated Admin API.
//!
//! The archive is asynchronous by contract, so a test waits for it the way an operator does rather
//! than assuming the write landed with the turn. Nothing here asserts on an internal counter for
//! something a client can see; the counters are read only for the drop classes a turn cannot show.
//!
//! Verifies:
//! - Capture: opt-in capture, conversation archiving, tool argument exclusion, interrupts, template attribution, and admission gating.
//! - Drop Policy: bounded hand-off, queue saturation, and database refusal handling without dropping voice turns.
//! - Retention: startup and scheduled cleanup, and non-blocking retry on write contention.
//! - Admin API: authenticated read & purge endpoints, validation, audit logging, and open session dialogue isolation.

#[path = "transcript_archive/harness.rs"]
mod harness;

#[path = "transcript_archive/capture.rs"]
mod capture;

#[path = "transcript_archive/drop_policy.rs"]
mod drop_policy;

#[path = "transcript_archive/retention.rs"]
mod retention;

#[path = "transcript_archive/admin_api.rs"]
mod admin_api;
