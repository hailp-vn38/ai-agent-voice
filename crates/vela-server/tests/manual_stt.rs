//! Manual STT and Voice Session integration tests.
//!
//! Verifies:
//! - Manual Mode: final text history commit, STT message enqueue, empty/failed discard, bounded eviction, overflow handling.
//! - Rearm & Duplicate Start: repeated start preservation, VAD lease pinning, abort rearming.
//! - Auto Cycle: VAD speech trigger, pre-roll retention, stale event isolation.
//! - Turn Control: capacity denial fail-safe, late replacement invalidation.

#[path = "manual_stt/harness.rs"]
mod harness;

#[path = "manual_stt/manual.rs"]
mod manual;

#[path = "manual_stt/rearm.rs"]
mod rearm;

#[path = "manual_stt/auto_cycle.rs"]
mod auto_cycle;

#[path = "manual_stt/turn_control.rs"]
mod turn_control;
