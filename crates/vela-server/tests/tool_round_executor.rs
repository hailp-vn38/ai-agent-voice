//! The shared Tool-round Executor, observed at the boundary a Voice Protocol Client can see.
//!
//! Every test here drives a real WebSocket session with a real Device MCP peer and a real External
//! MCP server, and asserts on the observable order of what the model asked for, what the two
//! transports were actually called with, and what the next LLM request was given back.  Nothing
//! asserts on a private field, and nothing needs a real model.
//!
//! The one thing this file deliberately does not prove is the late-response *discard* path and the
//! counters that go with cancellation.  Over a socket, a response that has been posted and an
//! abort that has been sent race inside a one millisecond actor tick, so which of the two the
//! executor sees first is not something a black-box test gets to choose.  Those two are proven
//! where they can be made deterministic: on the actor itself, in `session::actor::tools`.

#[path = "tool_round_executor/harness.rs"]
mod harness;

#[path = "tool_round_executor/ordering.rs"]
mod ordering;

#[path = "tool_round_executor/caps.rs"]
mod caps;

#[path = "tool_round_executor/concurrency.rs"]
mod concurrency;

#[path = "tool_round_executor/cancellation.rs"]
mod cancellation;

#[path = "tool_round_executor/failure.rs"]
mod failure;

#[path = "tool_round_executor/review.rs"]
mod review;
