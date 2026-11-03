//! Speech Output Tracer integration tests.
//!
//! Verifies:
//! - Pacing & Delivery: canonical Opus frame pacing, queue retries, urgent abort, and listen-arm retention.
//! - Sentence Streaming: LLM text delta chunking, Vietnamese sentence boundaries, backpressure, and failure fences.
//! - WebSocket Protocol: full-stack TTS lifecycle events, streaming text synchronization, and TTS failure fences.
//! - Direct Speech: admin direct speech injection, device presence, session collision, and rate limits.

mod support;

#[path = "speechoutput_tracer/harness.rs"]
mod harness;

#[path = "speechoutput_tracer/delivery.rs"]
mod delivery;

#[path = "speechoutput_tracer/streaming.rs"]
mod streaming;

#[path = "speechoutput_tracer/websocket.rs"]
mod websocket;

#[path = "speechoutput_tracer/direct_speech.rs"]
mod direct_speech;
