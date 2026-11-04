//! Ticket 09: the External MCP admission snapshot, proven through the admission boundary.
//!
//! Everything here goes through the boundary a Voice Session actually crosses: the application
//! state resolves one Effective Session Profile for a Device, and the WebSocket either upgrades or
//! does not.  The assertions are about what a session may call, what it may not, and what an
//! External MCP failure may never change.

#[path = "external_mcp_admission/harness.rs"]
mod harness;

pub use harness::*;

#[path = "external_mcp_admission/connection_latency.rs"]
mod connection_latency;

#[path = "external_mcp_admission/snapshot.rs"]
mod snapshot;

#[path = "external_mcp_admission/policy.rs"]
mod policy;

#[path = "external_mcp_admission/probes.rs"]
mod probes;
