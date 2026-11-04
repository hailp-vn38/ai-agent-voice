//! TTS Worker Runtime integration tests.
//!
//! Verifies:
//! - Bindings: logical views and binding injection, stream isolation, state preservation across segments.
//! - Execution: worker pooling, slot reuse, streaming PCM delivery, and pacing safety.
//! - Cancellation: detached cancellation, quarantine on grace exhaustion, acceptance timeout cleanup.
//! - Lifecycle: initialization and warmup failures, graceful shutdown and unload, quarantine on panic.

#[path = "tts_worker_runtime/harness.rs"]
mod harness;

#[path = "tts_worker_runtime/bindings.rs"]
mod bindings;

#[path = "tts_worker_runtime/execution.rs"]
mod execution;

#[path = "tts_worker_runtime/cancellation.rs"]
mod cancellation;

#[path = "tts_worker_runtime/lifecycle.rs"]
mod lifecycle;
