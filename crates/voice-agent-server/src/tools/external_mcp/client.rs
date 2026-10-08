//! The client that owns one MCP server's credential snapshot for the lifetime of one Voice
//! Session.
//!
//! Ownership is the whole point: the credential is resolved once, immediately before
//! `initialize`, and lives here — never in `SessionActor`, never in a log, never re-resolved on a
//! hot path.  Rotating it is therefore invisible to an open session and visible only to the next
//! admission.

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use reqwest::header::{HeaderName, HeaderValue};
use rmcp::{
    ClientHandler, RoleClient,
    model::{
        CallToolRequestParams, CallToolResponse, ClientCapabilities, ClientConfig, Implementation,
        PaginatedRequestParams, ProtocolVersion,
    },
    service::{RunningService, serve_client},
    transport::streamable_http_client::{
        StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
    },
};
use serde_json::Value;
use url::Url;

use crate::{
    config::{ExternalMcpLimitsConfig, ExternalMcpNetworkConfig},
    database::{
        external_mcp_policy,
        secrets::{SecretRef, SecretResolveError, SecretResolver},
    },
    telemetry::{CallOutcome, Telemetry},
};

use super::rmcp_adapter::PolicyHttpClient;
use super::transport::{
    ExternalMcpAuth, ExternalMcpCallLimiter, ExternalMcpError, canonical_static_headers,
    response_byte_cap, tool_result_byte_cap,
};

/// One tool exactly as the remote server described it.  It is untrusted until the manager has
/// capped and validated it, and never reaches a caller unvalidated.
#[derive(Clone, Debug, PartialEq)]
pub struct RawExternalTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// One `tools/list` page and the cursor that continues it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExternalToolsPage {
    pub tools: Vec<RawExternalTool>,
    pub next_cursor: Option<String>,
}

/// The canonical, bounded result of one successful `tools/call`.
#[derive(Clone, Debug, PartialEq)]
pub struct ExternalToolOutcome {
    /// A remote tool-level error is still a completed protocol response; the caller decides what
    /// it means for the turn.
    pub is_error: bool,
    pub content: String,
}

/// Why one stored server cannot produce a client.  The only detail is a secret resolution class:
/// neither a reference nor a value can appear in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectFailure {
    /// The credential did not resolve; the reason is a bounded class from
    /// [`SecretResolveError`], not anything derived from the reference.
    Secret(&'static str),
    /// The record cannot describe a usable destination, header set or authentication shape.
    Unusable,
}

/// What the remote server actually answered.  Every branch is content-free.
struct VoiceAgentRmcp;

impl ClientHandler for VoiceAgentRmcp {
    fn get_info(&self) -> ClientConfig {
        ClientConfig::new(
            ClientCapabilities::default(),
            Implementation::new("voice-agent-server", env!("CARGO_PKG_VERSION")),
        )
        .with_protocol_version(ProtocolVersion::V_2024_11_05)
    }
}

type RmcpSession = RunningService<RoleClient, VoiceAgentRmcp>;

/// One MCP server, as one Voice Session may use it: an immutable handle holding the destination,
/// the validated static headers, the typed credential and the protocol session id.
pub struct ExternalMcpClient {
    server_key: String,
    endpoint: Url,
    static_headers: Vec<(HeaderName, HeaderValue)>,
    auth: ExternalMcpAuth,
    connect_timeout: Duration,
    call_timeout: Duration,
    page_cap: usize,
    result_cap: usize,
    /// Process-owned, so every call this session makes reports into the same counters.
    telemetry: Arc<dyn Telemetry>,
    /// RMCP owns the negotiated session id, JSON-RPC request IDs, lifecycle and SSE parsing.
    rmcp: Option<RmcpSession>,
    policy_http: PolicyHttpClient,
}

/// The handle never renders a credential, a header value or the destination: a debug rendering of
/// a session's MCP snapshot has to be safe to log.
impl From<Option<ExternalMcpError>> for CallOutcome {
    /// The bounded class one call's outcome is reported as.  It is a total mapping, so a class can
    /// never be missing for an error this client can produce.
    fn from(outcome: Option<ExternalMcpError>) -> Self {
        match outcome {
            None => Self::Success,
            Some(ExternalMcpError::ToolTimeout) => Self::Timeout,
            Some(ExternalMcpError::ToolUnavailable) => Self::Unavailable,
            Some(ExternalMcpError::ToolAuthFailed) => Self::AuthFailed,
            Some(ExternalMcpError::ToolInvalidResponse) => Self::InvalidResponse,
            Some(_) => Self::ProtocolError,
        }
    }
}

impl std::fmt::Debug for ExternalMcpClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ExternalMcpClient")
            .field("server_key", &self.server_key)
            .field("scheme", &self.endpoint.scheme())
            .field("static_header_count", &self.static_headers.len())
            .field("auth", &self.auth)
            .field("call_timeout", &self.call_timeout)
            .field("initialized", &self.rmcp.is_some())
            .finish()
    }
}

impl ExternalMcpClient {
    /// Builds a client and resolves its credential exactly once.
    ///
    /// The stored record is revalidated here rather than trusted: it came out of a database an
    /// operator writes, so the destination, the header names and the authentication shape all
    /// have to hold again at the moment a session is about to reach them.
    #[allow(clippy::too_many_arguments)]
    pub fn connect(
        server_key: &str,
        endpoint: &str,
        headers_json: &str,
        auth_type: &str,
        auth_header_name: Option<&str>,
        secret_ref: Option<&SecretRef>,
        connect_timeout: Duration,
        call_timeout: Duration,
        http: reqwest::Client,
        network: ExternalMcpNetworkConfig,
        limits: &ExternalMcpLimitsConfig,
        telemetry: Arc<dyn Telemetry>,
        secrets: &dyn SecretResolver,
    ) -> Result<Self, ConnectFailure> {
        let usable = || ConnectFailure::Unusable;
        let endpoint = Url::parse(endpoint).map_err(|_| usable())?;
        if !external_mcp_policy::valid_desired_url(endpoint.as_str(), &network) {
            return Err(usable());
        }
        let static_headers = canonical_static_headers(
            &serde_json::from_str::<Value>(headers_json)
                .unwrap_or(Value::Object(Default::default())),
        )
        .map_err(|_| usable())?;
        let Some(page_cap) = response_byte_cap(limits) else {
            return Err(usable());
        };
        let auth = resolve_auth(auth_type, auth_header_name, secret_ref, secrets)?;
        let policy_http = PolicyHttpClient::new(
            endpoint.clone(),
            http.clone(),
            response_byte_cap(limits).ok_or_else(usable)?,
        );
        Ok(Self {
            server_key: server_key.to_owned(),
            endpoint,
            static_headers,
            auth,
            connect_timeout,
            call_timeout,
            page_cap,
            result_cap: tool_result_byte_cap(limits),
            telemetry,
            rmcp: None,
            policy_http,
        })
    }

    pub fn server_key(&self) -> &str {
        &self.server_key
    }

    /// The per-call bound the owning session installed at admission.  A turn's own execution
    /// budget may make an actual call shorter, never longer.
    pub fn call_timeout(&self) -> Duration {
        self.call_timeout
    }

    /// The process-owned sink this server's calls report into.
    ///
    /// A caller that has to report something only a session can observe — a call dropped by
    /// cancellation, a response that arrived after its turn ended — reports it here, so those
    /// observations land in the same counters as the calls themselves instead of in a second,
    /// session-scoped tally.
    pub fn telemetry(&self) -> &Arc<dyn Telemetry> {
        &self.telemetry
    }

    /// `initialize`, then the required `initialized` notification.
    ///
    /// Mutating because this is the only moment the handle changes: afterwards it is the immutable
    /// snapshot the session keeps, credential and protocol session included.
    pub async fn initialize(&mut self) -> Result<(), ExternalMcpError> {
        let (bearer, typed_header) = self.auth.rmcp_parts();
        let mut headers = HashMap::from_iter(self.static_headers.iter().cloned());
        if let Some((name, value)) = typed_header {
            headers.insert(name, value);
        }
        let mut config =
            StreamableHttpClientTransportConfig::with_uri(self.endpoint.as_str().to_owned());
        config.auth_header = bearer;
        config.custom_headers = headers;
        config.max_concurrent_requests = 1;
        // Modern Streamable HTTP permits stateless JSON responses as well as servers that mint an
        // `mcp-session-id`; RMCP retains either lifecycle without falling back to legacy transport.
        config.allow_stateless = true;
        config.reinit_on_expired_session = false;
        config.control_request_timeout = self.connect_timeout;
        config.session_recovery_timeout = self.connect_timeout;
        config.max_sse_event_size = self.page_cap;
        let transport =
            StreamableHttpClientTransport::with_client(self.policy_http.clone(), config);
        self.rmcp = Some(
            tokio::time::timeout(
                self.connect_timeout,
                serve_client(VoiceAgentRmcp, transport),
            )
            .await
            .map_err(|_| ExternalMcpError::InitializeTimeout)?
            .map_err(|_| {
                if self.policy_http.take_auth_failed() {
                    ExternalMcpError::InitializeAuthFailed
                } else {
                    ExternalMcpError::InitializeFailed
                }
            })?,
        );
        Ok(())
    }

    /// One page of the catalog, in remote order, with the cursor that continues it.  A truncated
    /// or partial page is never produced: an unusable page costs the server its tools.
    pub async fn list_tools_page(
        &self,
        cursor: Option<&str>,
    ) -> Result<ExternalToolsPage, ExternalMcpError> {
        let session = self
            .rmcp
            .as_ref()
            .ok_or(ExternalMcpError::ToolsListInvalid)?;
        let params = cursor
            .filter(|value| !value.is_empty())
            .map(|value| PaginatedRequestParams::default().with_cursor(Some(value.to_owned())));
        let page = tokio::time::timeout(self.call_timeout, session.peer().list_tools(params))
            .await
            .map_err(|_| ExternalMcpError::ToolsListTimeout)?
            .map_err(|_| ExternalMcpError::ToolsListInvalid)?;
        let result = serde_json::to_value(page).map_err(|_| ExternalMcpError::ToolsListInvalid)?;
        parse_tools_page(&result)
    }

    /// One bounded outbound `tools/call` attempt, and never a second one.
    ///
    /// A side effect may already have happened remotely before a timeout or a dropped connection,
    /// so retrying is not a recovery strategy.  A call that times out, is refused, is
    /// unauthenticated or answers unusably is a complete, typed outcome about one invocation and
    /// says nothing about the catalog.
    ///
    /// `budget` is what the caller's turn has left.  It bounds the wait for a call permit as well
    /// as the request itself, so a saturated server cannot spend time this turn does not have — and
    /// when the budget runs out before a permit exists, the request is never sent.
    pub async fn call_tool_guarded(
        &self,
        limiter: &ExternalMcpCallLimiter,
        tool: &super::registry::ResolvedExternalTool,
        arguments: &Value,
        budget: Duration,
        guard: &crate::database::tool_security::ExternalToolGuard,
    ) -> Result<ExternalToolOutcome, ExternalMcpError> {
        // The permit is taken before the request exists.  A caller that runs out of budget waiting
        // sends nothing, which is its own observation and not a call.
        let _permit = match limiter.acquire_within(&self.server_key, budget).await {
            Some(permit) => permit,
            None => {
                self.telemetry.call_limiter_rejected(&self.server_key);
                return Err(ExternalMcpError::ToolUnavailable);
            }
        };
        let started = Instant::now();
        let outcome = async {
            let session = self
                .rmcp
                .as_ref()
                .ok_or(ExternalMcpError::ToolUnavailable)?;
            let arguments = arguments
                .as_object()
                .cloned()
                .ok_or(ExternalMcpError::ToolInvalidResponse)?;
            let publication = guard.database.tool_security.publication.read().await;
            if !guard.allows(&self.server_key, &tool.original_name).await {
                return Err(ExternalMcpError::ToolUnavailable);
            }
            let params =
                CallToolRequestParams::new(tool.original_name.clone()).with_arguments(arguments);
            // Start the outbound operation atomically with the authority check, then release
            // publication coordination; revocation never waits for the remote response.
            let mut operation = Box::pin(session.call_tool_once(params));
            let first = std::future::poll_fn(|cx| {
                std::task::Poll::Ready(match std::future::Future::poll(operation.as_mut(), cx) {
                    std::task::Poll::Ready(value) => Some(value),
                    std::task::Poll::Pending => None,
                })
            })
            .await;
            drop(publication);
            let response = tokio::time::timeout(budget.min(self.call_timeout), async {
                match first {
                    Some(value) => value,
                    None => operation.await,
                }
            })
            .await
            .map_err(|_| ExternalMcpError::ToolTimeout)?
            .map_err(|error| {
                if self.policy_http.take_auth_failed() {
                    ExternalMcpError::ToolAuthFailed
                } else if self.policy_http.take_invalid_response() {
                    ExternalMcpError::ToolInvalidResponse
                } else {
                    match error {
                        // RMCP received a response for this request but it cannot be the
                        // `tools/call` result our protocol contract requires.  The request did
                        // reach the peer (the regression asserts that), so this is not network
                        // unavailability.
                        rmcp::service::ServiceError::UnexpectedResponse => {
                            ExternalMcpError::ToolInvalidResponse
                        }
                        rmcp::service::ServiceError::Timeout { .. } => {
                            ExternalMcpError::ToolTimeout
                        }
                        _ => ExternalMcpError::ToolUnavailable,
                    }
                }
            })?;
            if self.policy_http.take_auth_failed() {
                return Err(ExternalMcpError::ToolAuthFailed);
            }
            if self.policy_http.take_unavailable() {
                return Err(ExternalMcpError::ToolUnavailable);
            }
            let CallToolResponse::Complete(result) = response else {
                return Err(ExternalMcpError::ToolProtocolError);
            };
            canonical_tool_result(
                &serde_json::to_value(result).map_err(|_| ExternalMcpError::ToolInvalidResponse)?,
                self.result_cap,
            )
        }
        .await;
        self.telemetry.call_finished(
            &self.server_key,
            CallOutcome::from(outcome.as_ref().err().copied()),
            started.elapsed(),
        );
        outcome
    }
}

/// Typed authentication resolution.
///
/// A reference that does not resolve never reaches the network: the server is excluded as if it
/// were unavailable, and the reported reason is a bounded class rather than anything derived from
/// the reference or the value.
fn resolve_auth(
    auth_type: &str,
    auth_header_name: Option<&str>,
    secret_ref: Option<&SecretRef>,
    secrets: &dyn SecretResolver,
) -> Result<ExternalMcpAuth, ConnectFailure> {
    match (auth_type, secret_ref) {
        ("none", _) => Ok(ExternalMcpAuth::None),
        // A typed auth with no reference names a credential the deployment never configured.
        // That is a missing secret, not a malformed record, and it is reported as such.
        ("bearer", None) | ("header", None) => Err(ConnectFailure::Secret("secret_missing")),
        ("bearer", Some(reference)) => secrets
            .resolve(reference)
            .map(ExternalMcpAuth::Bearer)
            .map_err(|error| ConnectFailure::Secret(secret_failure_reason(error))),
        ("header", Some(reference)) => {
            let name = auth_header_name
                .and_then(|name| HeaderName::from_bytes(name.to_ascii_lowercase().as_bytes()).ok())
                .ok_or(ConnectFailure::Unusable)?;
            // The typed header is a credential path, so it is held to the same rule a stored
            // static header is: it may not claim a protected or malformed name.
            canonical_static_headers(&serde_json::json!({ name.as_str(): "x" }))
                .map_err(|_| ConnectFailure::Unusable)?;
            secrets
                .resolve(reference)
                .map(|value| ExternalMcpAuth::Header(name, value))
                .map_err(|error| ConnectFailure::Secret(secret_failure_reason(error)))
        }
        _ => Err(ConnectFailure::Unusable),
    }
}

/// The bounded class a credential failure is reported as.  `SecretResolveError` itself stays
/// internal so nothing derived from a reference or a value can reach a diagnostic.
fn secret_failure_reason(error: SecretResolveError) -> &'static str {
    match error {
        SecretResolveError::Invalid => "secret_invalid",
        SecretResolveError::Unavailable => "secret_resolver_unavailable",
    }
}

/// Strictly parses one `tools/list` result.  A partially usable page is refused: silently dropping
/// an entry the model would have expected is worse than losing the server's tools.
fn parse_tools_page(result: &Value) -> Result<ExternalToolsPage, ExternalMcpError> {
    let object = result
        .as_object()
        .ok_or(ExternalMcpError::ToolsListInvalid)?;
    let entries = object
        .get("tools")
        .and_then(Value::as_array)
        .ok_or(ExternalMcpError::ToolsListInvalid)?;
    let mut tools = Vec::with_capacity(entries.len());
    for entry in entries {
        let tool = entry
            .as_object()
            .ok_or(ExternalMcpError::ToolsListInvalid)?;
        let name = tool
            .get("name")
            .and_then(Value::as_str)
            .ok_or(ExternalMcpError::ToolsListInvalid)?;
        let description = match tool.get("description") {
            None => String::new(),
            Some(value) => value
                .as_str()
                .ok_or(ExternalMcpError::ToolsListInvalid)?
                .to_owned(),
        };
        let input_schema = tool
            .get("inputSchema")
            .filter(|schema| schema.is_object())
            .ok_or(ExternalMcpError::ToolsListInvalid)?;
        tools.push(RawExternalTool {
            name: name.to_owned(),
            description,
            input_schema: input_schema.clone(),
        });
    }
    let next_cursor = object
        .get("nextCursor")
        .map(|value| {
            value
                .as_str()
                .filter(|cursor| !cursor.is_empty())
                .map(str::to_owned)
                .ok_or(ExternalMcpError::ToolsListInvalid)
        })
        .transpose()?;
    Ok(ExternalToolsPage { tools, next_cursor })
}

/// Canonicalizes a successful `tools/call` result under a hard byte cap.
///
/// The result is untrusted too: only text and structured JSON are representable, every other
/// content kind — images, audio, embedded resources, arbitrary MIME — refuses the whole result
/// rather than being dropped from it, and nothing is ever truncated to fit.
fn canonical_tool_result(
    result: &Value,
    cap: usize,
) -> Result<ExternalToolOutcome, ExternalMcpError> {
    let object = result
        .as_object()
        .ok_or(ExternalMcpError::ToolInvalidResponse)?;
    let is_error = match object.get("isError") {
        None => false,
        Some(value) => value
            .as_bool()
            .ok_or(ExternalMcpError::ToolInvalidResponse)?,
    };
    // Whatever else the document carries, every content item has to be text.  One unsupported
    // kind anywhere refuses the whole result: keeping the text and dropping the rest would hand
    // the model a result the server never described.
    let text = match object.get("content") {
        None => Vec::new(),
        Some(items) => {
            let items = items
                .as_array()
                .ok_or(ExternalMcpError::ToolInvalidResponse)?;
            let mut text = Vec::with_capacity(items.len());
            for item in items {
                let item = item
                    .as_object()
                    .ok_or(ExternalMcpError::ToolInvalidResponse)?;
                if item.get("type").and_then(Value::as_str) != Some("text") {
                    return Err(ExternalMcpError::ToolInvalidResponse);
                }
                text.push(
                    item.get("text")
                        .and_then(Value::as_str)
                        .ok_or(ExternalMcpError::ToolInvalidResponse)?,
                );
            }
            text
        }
    };
    let content = match object.get("structuredContent") {
        Some(structured) => {
            if !structured.is_object() {
                return Err(ExternalMcpError::ToolInvalidResponse);
            }
            serde_json::to_string(structured).map_err(|_| ExternalMcpError::ToolInvalidResponse)?
        }
        None => {
            if text.is_empty() {
                return Err(ExternalMcpError::ToolInvalidResponse);
            }
            text.join("\n")
        }
    };
    if content.len() > cap {
        return Err(ExternalMcpError::ToolInvalidResponse);
    }
    Ok(ExternalToolOutcome { is_error, content })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_page_must_describe_every_tool_it_claims() {
        let page = parse_tools_page(&serde_json::json!({
            "tools": [
                {"name": "light", "description": "on", "inputSchema": {"type": "object"}},
                {"name": "dim", "inputSchema": {"type": "object"}}
            ],
            "nextCursor": "page-2"
        }))
        .expect("a well formed page parses");
        assert_eq!(page.tools.len(), 2);
        assert_eq!(page.tools[1].description, "");
        assert_eq!(page.next_cursor.as_deref(), Some("page-2"));

        for malformed in [
            serde_json::json!({"tools": "not-an-array"}),
            serde_json::json!({"tools": [{"description": "no name", "inputSchema": {}}]}),
            serde_json::json!({"tools": [{"name": "light"}]}),
            serde_json::json!({"tools": [{"name": "light", "inputSchema": []}]}),
            serde_json::json!({"tools": [{"name": "light", "inputSchema": {}, "description": 7}]}),
            serde_json::json!({"tools": [], "nextCursor": ""}),
            serde_json::json!([]),
        ] {
            assert_eq!(
                parse_tools_page(&malformed),
                Err(ExternalMcpError::ToolsListInvalid),
                "{malformed} must not become a partial catalog"
            );
        }
    }

    #[test]
    fn only_text_or_structured_json_survives_the_result_allowlist() {
        let cap = 4_096;
        assert_eq!(
            canonical_tool_result(
                &serde_json::json!({"content": [{"type": "text", "text": "a"}, {"type": "text", "text": "b"}]}),
                cap
            ),
            Ok(ExternalToolOutcome {
                is_error: false,
                content: "a\nb".to_owned()
            })
        );
        assert_eq!(
            canonical_tool_result(
                &serde_json::json!({"content": [{"type": "text", "text": "ignored"}], "structuredContent": {"ok": true}}),
                cap
            )
            .expect("structured content is representable")
            .content,
            "{\"ok\":true}"
        );
        assert_eq!(
            canonical_tool_result(
                &serde_json::json!({"content": [{"type": "text", "text": "boom"}], "isError": true}),
                cap
            )
            .expect("a tool-level error is a completed response"),
            ExternalToolOutcome {
                is_error: true,
                content: "boom".to_owned()
            }
        );

        for refused in [
            serde_json::json!({"content": [{"type": "image", "data": "x", "mimeType": "image/png"}]}),
            serde_json::json!({"content": [{"type": "audio", "data": "x"}]}),
            serde_json::json!({"content": [{"type": "resource", "resource": {}}]}),
            serde_json::json!({"content": [{"type": "resource_link", "uri": "x"}]}),
            serde_json::json!({"content": [{"type": "text", "text": "ok"}, {"type": "image", "data": "x"}]}),
            serde_json::json!({"content": []}),
            serde_json::json!({"structuredContent": "not-an-object"}),
            serde_json::json!({}),
        ] {
            assert_eq!(
                canonical_tool_result(&refused, cap),
                Err(ExternalMcpError::ToolInvalidResponse),
                "{refused} must refuse the whole result"
            );
        }
    }

    #[test]
    fn an_oversized_result_is_refused_rather_than_truncated() {
        let oversized = serde_json::json!({"content": [{"type": "text", "text": "x".repeat(64)}]});
        assert_eq!(
            canonical_tool_result(&oversized, 63),
            Err(ExternalMcpError::ToolInvalidResponse)
        );
        assert!(canonical_tool_result(&oversized, 64).is_ok());
    }

    #[test]
    fn a_typed_auth_shape_is_resolved_once_and_never_prints_its_value() {
        use crate::database::secrets::SecretValue;
        struct Fixed;
        impl SecretResolver for Fixed {
            fn resolve(&self, _: &SecretRef) -> Result<SecretValue, SecretResolveError> {
                Ok(SecretValue::new("s3cr3t".into()))
            }
        }
        struct Missing;
        impl SecretResolver for Missing {
            fn resolve(&self, _: &SecretRef) -> Result<SecretValue, SecretResolveError> {
                Err(SecretResolveError::Unavailable)
            }
        }
        let reference = SecretRef::parse("WEATHER_TOKEN".into()).expect("opaque reference");
        let bearer = resolve_auth("bearer", None, Some(&reference), &Fixed)
            .expect("a bearer reference resolves");
        assert_eq!(format!("{bearer:?}"), "ExternalMcpAuth(bearer, [REDACTED])");
        assert_eq!(
            resolve_auth("bearer", None, Some(&reference), &Missing).err(),
            Some(ConnectFailure::Secret("secret_resolver_unavailable"))
        );
        assert!(matches!(
            resolve_auth("none", None, None, &Missing),
            Ok(ExternalMcpAuth::None)
        ));
        assert_eq!(
            resolve_auth("bearer", None, None, &Fixed).err(),
            Some(ConnectFailure::Secret("secret_missing")),
            "a typed auth with no reference names a credential the deployment never configured"
        );
        assert_eq!(
            resolve_auth("header", Some("authorization"), Some(&reference), &Fixed).err(),
            Some(ConnectFailure::Unusable),
            "a typed auth header may not claim a protected name"
        );
        assert_eq!(
            resolve_auth("header", None, Some(&reference), &Fixed).err(),
            Some(ConnectFailure::Unusable),
            "a typed header auth without a header name is not a usable credential path"
        );
        assert_eq!(
            resolve_auth("unknown", None, None, &Fixed).err(),
            Some(ConnectFailure::Unusable)
        );
    }
}
