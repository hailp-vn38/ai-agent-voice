//! Ticket 12: readiness, ordered shutdown and operational degradation, proven through the
//! boundaries an operator actually observes.
//!
//! Everything here goes through `/health`, `/ready`, the WebSocket admission boundary and the
//! application lifecycle.  The assertions are about what a process tells its orchestrator and what
//! it accepts after a shutdown signal, never about which internal field was read to decide.

#[path = "support/admin_web.rs"]
mod admin_web;

#[path = "lifecycle/harness.rs"]
mod harness;

#[path = "lifecycle/readiness.rs"]
mod readiness;

#[path = "lifecycle/gate.rs"]
mod gate;

#[path = "lifecycle/drain.rs"]
mod drain;

#[path = "lifecycle/managed.rs"]
mod managed;
