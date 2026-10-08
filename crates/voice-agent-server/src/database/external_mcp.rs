//! Database read side of External MCP admission.
//!
//! This is the only SQL behind External MCP discovery.  It copies the Agent's enabled bindings
//! and their desired server configuration out of SQLite before any network work begins, so a
//! the credential name is derived at admission and resolved once off the hot path.

use thiserror::Error;

use super::{Database, DatabaseError, map_sqlx_error};

/// One enabled MCP server an Agent publishes, copied verbatim from its desired configuration.
///
/// The optional in-memory reference is derived from server identity and auth mode;
/// no credential reference or value is read from SQLite.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmittedMcpServer {
    pub id: i64,
    pub key: String,
    pub url: String,
    pub headers_json: String,
    pub auth_type: String,
    pub auth_header_name: Option<String>,
    pub secret_ref: Option<String>,
    pub connect_timeout_ms: i64,
    pub request_timeout_ms: i64,
    pub revision: i64,
}

#[derive(Debug, Error)]
pub enum McpAdmissionError {
    #[error("mcp_binding_unavailable")]
    Unavailable,
}

impl From<DatabaseError> for McpAdmissionError {
    fn from(_: DatabaseError) -> Self {
        Self::Unavailable
    }
}

impl Database {
    /// Every enabled binding the Agent publishes, in one bounded read.
    ///
    /// Ordering by server key makes the snapshot independent of the order bindings were created
    /// in, which is what lets collision and aggregate-cap handling stay deterministic.  A
    /// disabled binding or a disabled server is not an Agent intent this session should act on.
    pub async fn agent_mcp_servers(
        &self,
        agent_id: i64,
    ) -> Result<Vec<AdmittedMcpServer>, McpAdmissionError> {
        let rows = sqlx::query_as::<
            _,
            (
                i64,
                String,
                String,
                String,
                String,
                Option<String>,
                i64,
                i64,
                i64,
            ),
        >(
            "SELECT m.id, m.key, m.url, m.headers_json, m.auth_type, m.auth_header_name, \
                    m.connect_timeout_ms, m.request_timeout_ms, m.revision \
             FROM agent_mcp_bindings b JOIN mcp_servers m ON m.id = b.mcp_server_id \
             WHERE b.agent_id = ? AND b.enabled = 1 AND m.enabled = 1 ORDER BY m.key",
        )
        .bind(agent_id)
        .fetch_all(&self.pool)
        .await
        .map_err(map_sqlx_error)?;
        Ok(rows
            .into_iter()
            .map(
                |(
                    id,
                    key,
                    url,
                    headers_json,
                    auth_type,
                    auth_header_name,
                    connect_timeout_ms,
                    request_timeout_ms,
                    revision,
                )| AdmittedMcpServer {
                    id,
                    key,
                    url,
                    headers_json,
                    auth_type,
                    auth_header_name,
                    secret_ref: super::secrets::mcp_secret_env(&key, &auth_type),
                    connect_timeout_ms,
                    request_timeout_ms,
                    revision,
                },
            )
            .collect())
    }
}
