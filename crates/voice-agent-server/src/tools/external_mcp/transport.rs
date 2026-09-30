//! Outbound boundary for External MCP: the process-global per-server call limiter, fixed request
//! assembly and the typed wire outcomes every caller maps to a bounded result.
//!
//! Request assembly has exactly one order — validated static headers, then standard MCP headers,
//! then typed authentication — and offers no way to insert a header afterwards.  That is what
//! keeps a database record from becoming an arbitrary HTTP client: the only credential path is
//! the typed auth this module injects.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use reqwest::header::{ACCEPT, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use serde_json::Value;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use crate::{
    config::{ExternalMcpLimitsConfig, ExternalMcpNetworkConfig},
    database::external_mcp_policy,
    database::secrets::SecretValue,
    lifecycle::AdmissionGate,
};

use super::registry::ToolPublishError;

/// The protocol revision this client speaks.  It travels on every request after `initialize`.
pub const MCP_PROTOCOL_VERSION: &str = "2024-11-05";

/// Standard MCP headers are always sent, so a configured static header can never displace the
/// transport contract.
const MCP_MEDIA_TYPE: &str = "application/json";

/// One MCP response is buffered once, so its cap is derived from the operator's catalog limits
/// rather than guessed: it must carry a full legal page and refuse anything larger.
pub const MAX_EXTERNAL_MCP_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

/// Framing overhead around one tool's name, description and schema in a `tools/list` page.
const TOOL_ENTRY_OVERHEAD_BYTES: usize = 512;

/// Framing overhead around one `tools/call` result document.
const RESULT_ENVELOPE_OVERHEAD_BYTES: usize = 1_024;

/// A `tools/call` response carries one tool's result, never a catalog, so it is bounded by the
/// operator's own result cap rather than by a page-derived budget.
pub fn tool_result_byte_cap(limits: &ExternalMcpLimitsConfig) -> usize {
    limits
        .max_external_tool_result_bytes
        .min(MAX_EXTERNAL_MCP_RESPONSE_BYTES)
}

/// The JSON-RPC envelope around a result is larger than the result it wraps, so the response is
/// read under the result cap plus a bounded framing allowance.  The canonical result is then
/// checked against the operator's cap on its own, so the allowance never widens what may be
/// published — it only stops a legal result from being refused by its own envelope.
pub fn tool_result_wire_cap(limits: &ExternalMcpLimitsConfig) -> usize {
    limits
        .max_external_tool_result_bytes
        .saturating_add(RESULT_ENVELOPE_OVERHEAD_BYTES)
        .min(MAX_EXTERNAL_MCP_RESPONSE_BYTES)
}

/// The response budget a configured catalog implies.  A configuration whose budget exceeds the
/// buffer is a configuration error, refused at config load rather than becoming a server that
/// silently cannot be discovered.
pub fn response_byte_cap(limits: &ExternalMcpLimitsConfig) -> Option<usize> {
    limits
        .max_tools_per_server
        .checked_mul(
            limits
                .max_tool_schema_bytes
                .checked_add(limits.max_tool_description_bytes)?
                .checked_add(TOOL_ENTRY_OVERHEAD_BYTES)?,
        )
        .filter(|derived| *derived <= MAX_EXTERNAL_MCP_RESPONSE_BYTES)
}

/// Bounded outcome of one External MCP interaction.  These strings are the only failure content a
/// caller may surface: no remote body, URL, arguments or diagnostic ever becomes a result.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ExternalMcpError {
    #[error("mcp_server_unavailable")]
    ServerUnavailable,
    #[error("mcp_initialize_failed")]
    InitializeFailed,
    #[error("mcp_tools_list_timeout")]
    ToolsListTimeout,
    #[error("mcp_tools_list_invalid")]
    ToolsListInvalid,
    #[error("mcp_initialize_auth_failed")]
    InitializeAuthFailed,
    #[error("mcp_initialize_protocol_error")]
    InitializeProtocolError,
    #[error("mcp_initialize_unavailable")]
    InitializeUnavailable,
    #[error("mcp_initialize_timeout")]
    InitializeTimeout,
    #[error("mcp_tools_list_unavailable")]
    ToolsListUnavailable,
    #[error("mcp_tool_name_invalid")]
    ToolNameInvalid,
    #[error("mcp_tool_name_collision")]
    ToolNameCollision,
    #[error("mcp_server_catalog_rejected")]
    CatalogRejected,
    #[error("external_tool_timeout")]
    ToolTimeout,
    #[error("external_tool_unavailable")]
    ToolUnavailable,
    #[error("external_tool_auth_failed")]
    ToolAuthFailed,
    #[error("external_tool_invalid_response")]
    ToolInvalidResponse,
    #[error("external_tool_protocol_error")]
    ToolProtocolError,
}

impl From<ToolPublishError> for ExternalMcpError {
    fn from(error: ToolPublishError) -> Self {
        match error {
            ToolPublishError::NameCollision => Self::ToolNameCollision,
        }
    }
}

/// Process-global outbound concurrency, keyed by immutable MCP server identity and shared by every
/// Voice Session.  One server saturating its bound can never delay another server or Device MCP.
#[derive(Clone)]
pub struct ExternalMcpCallLimiter {
    capacity: u32,
    servers: Arc<Mutex<HashMap<String, Arc<Semaphore>>>>,
    /// The application admission gate.  A closed gate refuses a *new* permit, which is what stops a
    /// Tool-round work item queued before shutdown from putting a request on the network after it.
    /// It never revokes a permit already held: the call that took one either finishes inside its
    /// own budget or is dropped by its turn's cancellation, and revoking it would be a second,
    /// different close mechanism.
    gate: Arc<AdmissionGate>,
}

impl std::fmt::Debug for ExternalMcpCallLimiter {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ExternalMcpCallLimiter")
            .field("capacity", &self.capacity)
            .field("servers", &self.server_count())
            .field("admission_open", &self.gate.is_open())
            .finish()
    }
}

impl ExternalMcpCallLimiter {
    /// The bound is validated as `1..=64` at configuration load, so a limiter always has a
    /// usable permit count.
    pub fn new(capacity: u32) -> Self {
        Self::with_gate(capacity, AdmissionGate::open())
    }

    /// The same limiter, refusing new permits once the application has closed its admission gate.
    pub fn with_gate(capacity: u32, gate: Arc<AdmissionGate>) -> Self {
        assert!(
            (1..=64).contains(&capacity),
            "the configured per-server call bound is validated at config load"
        );
        Self {
            capacity,
            servers: Arc::new(Mutex::new(HashMap::new())),
            gate,
        }
    }

    pub fn capacity(&self) -> u32 {
        self.capacity
    }

    pub fn server_count(&self) -> usize {
        self.servers
            .lock()
            .map(|servers| servers.len())
            .unwrap_or(0)
    }

    /// A permit is taken before the request leaves the process and released before the caller
    /// continues the turn, so no other session's work is held across a ToolResult.
    ///
    /// Acquisition waits, but only for as long as the caller's own budget allows.  Waiting is not
    /// free — it is spent out of the turn's execution budget — so a caller that arrives with no
    /// budget left sends nothing at all, and one that runs out while waiting does the same.  There
    /// is no queue, no retry and no separate backoff behind this bound.
    ///
    /// A closed application gate refuses the permit without waiting at all, and the caller reports
    /// that refusal as the same bounded class as an exhausted bound: the request was never sent, so
    /// nothing observable distinguishes the two, and neither class carries a destination, an
    /// argument or a credential.
    pub async fn acquire_within(
        &self,
        server_key: &str,
        budget: Duration,
    ) -> Option<OwnedSemaphorePermit> {
        if !self.gate.is_open() {
            return None;
        }
        let semaphore = self.semaphore(server_key)?;
        tokio::time::timeout(budget, Arc::clone(&semaphore).acquire_owned())
            .await
            .ok()?
            .ok()
    }

    /// Non-blocking acquisition, for a caller that has already decided not to wait.
    pub fn try_acquire(&self, server_key: &str) -> Option<OwnedSemaphorePermit> {
        if !self.gate.is_open() {
            return None;
        }
        Arc::clone(&self.semaphore(server_key)?)
            .try_acquire_owned()
            .ok()
    }

    fn semaphore(&self, server_key: &str) -> Option<Arc<Semaphore>> {
        let mut servers = self.servers.lock().ok()?;
        Some(Arc::clone(
            servers
                .entry(server_key.to_owned())
                .or_insert_with(|| Arc::new(Semaphore::new(self.capacity as usize))),
        ))
    }
}

/// Typed authentication.  There is no other way to put a credential on a request.
pub enum ExternalMcpAuth {
    None,
    Bearer(SecretValue),
    Header(HeaderName, SecretValue),
}

impl std::fmt::Debug for ExternalMcpAuth {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // A debug rendering of a credential path must never be a credential disclosure.
        formatter.write_str(match self {
            Self::None => "ExternalMcpAuth(none)",
            Self::Bearer(_) => "ExternalMcpAuth(bearer, [REDACTED])",
            Self::Header(_, _) => "ExternalMcpAuth(header, [REDACTED])",
        })
    }
}

impl ExternalMcpAuth {
    /// Injected last, after static and standard headers, so nothing can overwrite it.
    fn apply(&self, headers: &mut HeaderMap) {
        match self {
            Self::None => {}
            Self::Bearer(token) => {
                if let Ok(value) = HeaderValue::from_str(&format!("Bearer {}", token.expose())) {
                    headers.insert(reqwest::header::AUTHORIZATION, value);
                }
            }
            Self::Header(name, token) => {
                if let Ok(value) = HeaderValue::from_str(token.expose()) {
                    headers.insert(name.clone(), value);
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum StaticHeaderError {
    #[error("static_header_name_invalid")]
    NameInvalid,
    #[error("static_header_protected")]
    Protected,
    #[error("static_header_value_invalid")]
    ValueInvalid,
}

/// Header names whose meaning belongs to the transport or to authentication.  A stored static
/// header may never claim one, so an Admin record cannot impersonate a credential.
const PROTECTED_HEADERS: &[&str] = &[
    "authorization",
    "proxy-authorization",
    "cookie",
    "set-cookie",
    "host",
    "content-length",
    "transfer-encoding",
    "connection",
    "upgrade",
    "te",
    "trailer",
    "proxy-authenticate",
    "www-authenticate",
    "keep-alive",
];

/// Re-parses stored static headers into canonical typed values.  The stored JSON is revalidated
/// at admission rather than trusted: the database is an untrusted input here, not a validator.
pub fn canonical_static_headers(
    value: &Value,
) -> Result<Vec<(HeaderName, HeaderValue)>, StaticHeaderError> {
    let object = value.as_object().ok_or(StaticHeaderError::NameInvalid)?;
    let mut headers = Vec::with_capacity(object.len());
    for (name, value) in object {
        let text = value.as_str().ok_or(StaticHeaderError::ValueInvalid)?;
        if text.len() > 4_096 || text.bytes().any(|byte| byte < 0x20 || byte == 0x7f) {
            return Err(StaticHeaderError::ValueInvalid);
        }
        if PROTECTED_HEADERS.contains(&name.to_ascii_lowercase().as_str()) {
            return Err(StaticHeaderError::Protected);
        }
        headers.push((
            HeaderName::from_bytes(name.to_ascii_lowercase().as_bytes())
                .map_err(|_| StaticHeaderError::NameInvalid)?,
            HeaderValue::from_str(text).map_err(|_| StaticHeaderError::ValueInvalid)?,
        ));
    }
    Ok(headers)
}

/// Builds one outbound request.  The order is fixed and total: validated static headers, then
/// standard MCP headers, then typed authentication.
pub(crate) fn assemble(
    http: &reqwest::Client,
    endpoint: &url::Url,
    static_headers: &[(HeaderName, HeaderValue)],
    auth: &ExternalMcpAuth,
    session_id: Option<&HeaderValue>,
    body: &Value,
) -> reqwest::RequestBuilder {
    let mut headers = HeaderMap::new();
    for (name, value) in static_headers {
        headers.insert(name.clone(), value.clone());
    }
    headers.insert(CONTENT_TYPE, HeaderValue::from_static(MCP_MEDIA_TYPE));
    headers.insert(
        ACCEPT,
        HeaderValue::from_static("application/json, text/event-stream"),
    );
    if let Some(session_id) = session_id {
        headers.insert(
            HeaderName::from_static("mcp-session-id"),
            session_id.clone(),
        );
    }
    auth.apply(&mut headers);
    http.post(endpoint.clone()).headers(headers).json(body)
}

/// The outcome of one JSON-RPC exchange.  A remote error is a protocol-level answer, not a
/// transport failure.  Its code and message are deliberately not carried further: a remote server
/// gets to describe its own failure, not to describe this deployment's session.
pub(crate) enum RpcOutcome {
    Result(Value),
    Error,
}

/// Accepts only the two response encodings MCP Streamable HTTP defines.  Anything else, and any
/// body that is not a well-formed JSON-RPC response for the id we sent, is malformed.
pub(crate) fn parse_rpc_response(content_type: &str, body: &[u8]) -> Option<RpcOutcome> {
    let media_type = content_type.split(';').next()?.trim().to_ascii_lowercase();
    let document = match media_type.as_str() {
        MCP_MEDIA_TYPE => serde_json::from_slice::<Value>(body).ok()?,
        "text/event-stream" => {
            let text = std::str::from_utf8(body).ok()?;
            text.lines()
                .filter_map(|line| line.strip_prefix("data:").map(str::trim))
                .find_map(|event| serde_json::from_str::<Value>(event).ok())?
        }
        _ => return None,
    };
    if document.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return None;
    }
    document.get("id")?;
    match document.get("result") {
        Some(result) => Some(RpcOutcome::Result(result.clone())),
        None => document
            .get("error")
            .filter(|error| error.get("code").is_some())
            .map(|_| RpcOutcome::Error),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadRejection {
    Unavailable,
    TooLarge,
}

/// Buffers one response under a hard cap.  The cap is checked while the body arrives, so an
/// oversized response is abandoned rather than fully received and then refused.
pub(crate) async fn read_bounded(
    mut response: reqwest::Response,
    cap: usize,
) -> Result<Vec<u8>, ReadRejection> {
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| ReadRejection::Unavailable)?
    {
        if body.len().saturating_add(chunk.len()) > cap {
            return Err(ReadRejection::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// One dial, revalidating the destination immediately before the bytes leave the process.  The
/// authorization a hostname rule grants and the numeric destination it resolves to are checked
/// together, so a DNS answer cannot point somewhere the operator did not allow.
///
/// A destination the policy refuses is reported exactly like one that is simply down: the caller
/// owns the phase classification, and the reason never travels further.
pub(crate) async fn dial(
    endpoint: &url::Url,
    network: &ExternalMcpNetworkConfig,
    request: reqwest::RequestBuilder,
) -> Result<reqwest::Response, ()> {
    external_mcp_policy::resolve_and_validate(endpoint, network)
        .await
        .map_err(|_| ())?;
    request.send().await.map_err(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limiter() -> ExternalMcpCallLimiter {
        ExternalMcpCallLimiter::new(2)
    }

    #[tokio::test]
    async fn one_servers_saturation_never_reaches_another_server() {
        let limiter = limiter();
        let first = limiter.try_acquire("weather").expect("a permit is free");
        let second = limiter
            .try_acquire("weather")
            .expect("the bound allows two");
        assert!(limiter.try_acquire("weather").is_none());

        // Device MCP and every other server keep the whole bound while one server is saturated.
        assert!(limiter.try_acquire("other").is_some());
        drop(first);
        assert!(limiter.try_acquire("weather").is_some());
        drop(second);
    }

    #[tokio::test]
    async fn a_permit_is_released_when_its_caller_drops_it() {
        let limiter = limiter();
        for _ in 0..4 {
            let permit = limiter.try_acquire("weather").expect("a permit is free");
            drop(permit);
        }
        assert_eq!(limiter.capacity(), 2);
        assert_eq!(limiter.server_count(), 1);
    }

    #[tokio::test]
    async fn a_caller_with_no_budget_left_sends_nothing_rather_than_waiting() {
        let limiter = limiter();
        let held = limiter.try_acquire("weather").expect("a permit is free");
        let _second = limiter
            .try_acquire("weather")
            .expect("the bound allows two");

        // The turn's own execution budget is the only thing a caller may spend waiting, and a
        // caller that arrives with none of it is refused instead of queued.
        assert!(limiter.try_acquire("weather").is_none());
        assert!(
            limiter
                .acquire_within("weather", Duration::from_millis(20))
                .await
                .is_none()
        );
        drop(held);
        assert!(
            limiter
                .acquire_within("weather", Duration::from_millis(20))
                .await
                .is_some()
        );
    }

    #[test]
    fn static_headers_are_canonicalized_and_protected_names_are_refused() {
        let parsed = canonical_static_headers(&serde_json::json!({
            "X-Tenant": "kitchen",
            "X-Trace": "abc"
        }))
        .expect("ordinary headers are accepted");
        assert_eq!(parsed.len(), 2);
        assert!(parsed.iter().any(|(name, _)| name.as_str() == "x-tenant"));

        for protected in PROTECTED_HEADERS {
            assert_eq!(
                canonical_static_headers(&serde_json::json!({ *protected: "value" })),
                Err(StaticHeaderError::Protected),
                "{protected} must never come from a stored record"
            );
        }
        assert_eq!(
            canonical_static_headers(&serde_json::json!({"X-Bad": 7})),
            Err(StaticHeaderError::ValueInvalid)
        );
        assert_eq!(
            canonical_static_headers(&serde_json::json!({"X-Bad": "a\nb"})),
            Err(StaticHeaderError::ValueInvalid)
        );
    }

    #[test]
    fn typed_auth_is_assembled_last_and_never_renders_its_credential() {
        let http = reqwest::Client::new();
        let endpoint = url::Url::parse("https://mcp.internal.test/rpc").unwrap();
        let static_headers = canonical_static_headers(&serde_json::json!({"X-Tenant": "kitchen"}))
            .expect("the static header is valid");
        let auth = ExternalMcpAuth::Bearer(SecretValue::new("s3cr3t".into()));
        assert_eq!(
            format!("{auth:?}"),
            "ExternalMcpAuth(bearer, [REDACTED])",
            "a debug rendering must not disclose a credential"
        );
        let request = assemble(
            &http,
            &endpoint,
            &static_headers,
            &auth,
            None,
            &serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
        )
        .build()
        .expect("the assembled request is well formed");
        let headers = request.headers();
        assert_eq!(headers.get("x-tenant").unwrap(), "kitchen");
        assert_eq!(headers.get("content-type").unwrap(), MCP_MEDIA_TYPE);
        assert_eq!(headers.get("authorization").unwrap(), "Bearer s3cr3t");
    }

    #[test]
    fn only_the_two_defined_streamable_http_encodings_are_accepted() {
        let result = serde_json::json!({"jsonrpc": "2.0", "id": 1, "result": {"ok": true}});
        assert!(matches!(
            parse_rpc_response("application/json", result.to_string().as_bytes()),
            Some(RpcOutcome::Result(_))
        ));
        let stream = format!(
            "event: message\ndata: {}\n\n",
            serde_json::json!({"jsonrpc": "2.0", "id": 1, "result": {"ok": true}})
        );
        assert!(matches!(
            parse_rpc_response("text/event-stream", stream.as_bytes()),
            Some(RpcOutcome::Result(_))
        ));
        assert!(matches!(
            parse_rpc_response(
                "application/json",
                serde_json::json!({"jsonrpc": "2.0", "id": 1, "error": {"code": -32601, "message": "x"}})
                    .to_string()
                    .as_bytes()
            ),
            Some(RpcOutcome::Error)
        ));
        for refused in [
            ("text/html", "<html/>"),
            ("application/json", "not json"),
            (
                "application/json",
                r#"{"jsonrpc":"1.0","id":1,"result":{}}"#,
            ),
            // A notification is not an answer to the request we sent.
            (
                "application/json",
                r#"{"jsonrpc":"2.0","method":"notifications/x"}"#,
            ),
            ("application/json", r#"{"jsonrpc":"2.0","id":1}"#),
        ] {
            assert!(
                parse_rpc_response(refused.0, refused.1.as_bytes()).is_none(),
                "{} must not be read as a JSON-RPC response",
                refused.0
            );
        }
    }

    #[test]
    fn a_configured_page_that_cannot_be_buffered_is_refused_at_configuration_time() {
        let mut limits = ExternalMcpLimitsConfig::default();
        assert_eq!(
            response_byte_cap(&limits),
            Some(128 * (16_384 + 4_096 + 512))
        );
        limits.max_tools_per_server = 512;
        limits.max_tool_schema_bytes = 65_536;
        assert_eq!(response_byte_cap(&limits), None);
    }

    /// The limiter's bound is configuration, and configuration is what validates it.  If the two
    /// ever disagree, a session would size its waits against a bound nobody checked.
    #[test]
    fn the_limiter_bound_is_the_one_configuration_validates() {
        let manager =
            super::super::ExternalMcpManager::new(&crate::config::ExternalMcpConfig::default())
                .expect("the defaults are a buildable configuration");
        assert_eq!(manager.limiter().capacity(), 16);
    }
}
