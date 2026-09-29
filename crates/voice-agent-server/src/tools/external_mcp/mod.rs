pub mod client;
pub mod manager;
pub mod registry;
pub mod transport;

pub use client::{
    ConnectFailure, ExternalMcpClient, ExternalToolOutcome, ExternalToolsPage, RawExternalTool,
};
pub use manager::{
    ExternalMcpExclusion, ExternalMcpExclusionReason, ExternalMcpManager, ExternalMcpSnapshot,
    ResolvedExternalMcp, SessionExternalMcp,
};
pub use registry::{
    ExternalToolCatalog, MAX_EXTERNAL_TOOL_SEGMENT, ResolvedExternalTool, SchemaRejection,
    ToolOrigin, ToolPublishError, normalize_external_tool_segment, validate_external_tool_schema,
};
pub use transport::{
    ExternalMcpCallLimiter, ExternalMcpError, MAX_EXTERNAL_MCP_RESPONSE_BYTES, response_byte_cap,
};
