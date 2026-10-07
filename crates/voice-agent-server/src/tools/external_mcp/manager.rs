//! Application-owned External MCP resolution: the one place a fresh, bounded, fail-soft tool
//! snapshot is produced for a Voice Session admission.
//!
//! The HTTP transport, the connection pool and the per-server call limiter are shared across
//! sessions; the tools themselves never are.  Every admission walks `initialize` and the whole
//! paginated `tools/list` again, so a catalog a previous session verified is never authority for
//! this one — and a server that is slow, wrong or gone costs its own tools and nothing else.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::Arc,
    time::Duration,
};

use futures_util::future::join_all;
use serde_json::Value;

use crate::{
    config::{ExternalMcpConfig, ExternalMcpLimitsConfig},
    database::{
        AdmittedMcpServer,
        secrets::{SecretRef, SecretResolver},
    },
    lifecycle::AdmissionGate,
    telemetry::{Telemetry, TracingTelemetry},
};

use super::{
    client::{ConnectFailure, ExternalMcpClient},
    registry::{
        ExternalToolCatalog, ResolvedExternalTool, ToolOrigin, normalize_external_tool_segment,
        validate_external_tool_schema,
    },
    transport::{ExternalMcpCallLimiter, ExternalMcpError},
};

/// The immutable prefix every tool of one server carries.  Device MCP can never produce it.
const EXTERNAL_NAMESPACE: &str = "external";

/// One MCP server, as one Voice Session may use it for the rest of its life.
///
/// The handle owns the credential; the actor only ever holds this.  That split is what lets a
/// session outlive a secret rotation without a hot path ever re-resolving anything, and it is why
/// the snapshot can be handed to a Voice Session as plain immutable data.
///
/// Cloning shares one handle and therefore one credential: it is a second reference to the same
/// snapshot, never a second resolution.
#[derive(Clone)]
pub struct ResolvedExternalMcp {
    pub server_key: String,
    /// The `external.<normalized_server_key>` prefix shared by every published tool name.
    pub namespace: String,
    pub client: Arc<ExternalMcpClient>,
    pub tools: Arc<[ResolvedExternalTool]>,
    pub call_timeout: Duration,
    /// The process-global bound on concurrent outbound calls to this server.
    ///
    /// It travels with the server handle rather than with the session, so every session admitted to
    /// this server acquires from the same semaphore: a caller cannot reach an External MCP server
    /// without also passing through the deployment's concurrency bound for it.
    pub limiter: Arc<ExternalMcpCallLimiter>,
}

impl std::fmt::Debug for ResolvedExternalMcp {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // A snapshot debug rendering is emitted at admission, so it carries identity and shape
        // only: never a URL, a header, a credential or a tool argument.
        formatter
            .debug_struct("ResolvedExternalMcp")
            .field("server_key", &self.server_key)
            .field("namespace", &self.namespace)
            .field("tool_count", &self.tools.len())
            .field("call_timeout", &self.call_timeout)
            .finish()
    }
}

/// Why one bound server contributed no tools.  Bounded classes only, so a diagnostic can name a
/// server without saying anything about its destination, credentials or catalog.
///
/// One class name serves the log line, the exclusion and the metric label, so a counter and the
/// diagnostic beside it can never disagree about why a server published nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExternalMcpExclusionReason {
    ServerUnavailable,
    InitializeFailed,
    ToolsListTimeout,
    ToolsListInvalid,
    ToolNameCollision,
    CatalogRejected,
    ServerKeyCollision,
    /// A bounded secret resolution class — never the reference and never the value.
    SecretResolutionFailed(&'static str),
    SessionToolCapExceeded,
}

impl ExternalMcpExclusionReason {
    /// The bounded class this refusal is reported as.  It is the only spelling a metric label or a
    /// log line may use for it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ServerUnavailable => "mcp_server_unavailable",
            Self::InitializeFailed => "mcp_initialize_failed",
            Self::ToolsListTimeout => "mcp_tools_list_timeout",
            Self::ToolsListInvalid => "mcp_tools_list_invalid",
            Self::ToolNameCollision => "mcp_tool_name_collision",
            Self::CatalogRejected => "mcp_server_catalog_rejected",
            Self::ServerKeyCollision => "mcp_server_key_collision",
            Self::SecretResolutionFailed(reason) => {
                // The inner reason is itself a bounded class from the secret boundary.
                match reason {
                    "secret_missing" | "secret_invalid" | "secret_resolver_unavailable" => reason,
                    _ => "secret_resolver_unavailable",
                }
            }
            Self::SessionToolCapExceeded => "external_mcp_session_tool_cap_exceeded",
        }
    }
}

impl std::fmt::Display for ExternalMcpExclusionReason {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SecretResolutionFailed(_) => write!(
                formatter,
                "external_mcp_secret_resolution_failed:{}",
                self.as_str()
            ),
            class => formatter.write_str(class.as_str()),
        }
    }
}

impl std::error::Error for ExternalMcpExclusionReason {}

impl From<ExternalMcpError> for ExternalMcpExclusionReason {
    fn from(error: ExternalMcpError) -> Self {
        match error {
            ExternalMcpError::ServerUnavailable
            | ExternalMcpError::InitializeUnavailable
            | ExternalMcpError::ToolsListUnavailable => Self::ServerUnavailable,
            ExternalMcpError::InitializeFailed
            | ExternalMcpError::InitializeAuthFailed
            | ExternalMcpError::InitializeProtocolError => Self::InitializeFailed,
            ExternalMcpError::InitializeTimeout | ExternalMcpError::ToolsListTimeout => {
                Self::ToolsListTimeout
            }
            ExternalMcpError::ToolsListInvalid
            | ExternalMcpError::ToolNameInvalid
            | ExternalMcpError::ToolInvalidResponse => Self::ToolsListInvalid,
            ExternalMcpError::ToolNameCollision => Self::ToolNameCollision,
            _ => Self::CatalogRejected,
        }
    }
}

/// One server's absence from the snapshot, with the bounded reason it was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExternalMcpExclusion {
    pub server_key: String,
    pub reason: ExternalMcpExclusionReason,
}

/// Everything one admission learned about External MCP: the tools a session may call, and the
/// bounded reasons the rest did not make it.
#[derive(Clone, Debug, Default)]
pub struct ExternalMcpSnapshot {
    pub servers: Vec<ResolvedExternalMcp>,
    pub exclusions: Vec<ExternalMcpExclusion>,
}

impl ExternalMcpSnapshot {
    /// A session whose Agent binds no usable server still gets a valid snapshot; External MCP is
    /// never a reason to refuse a Voice Session.
    pub fn tool_count(&self) -> usize {
        self.servers.iter().map(|server| server.tools.len()).sum()
    }
}

/// The immutable External MCP catalog one Voice Session was admitted with.
///
/// This is a routing table, not a cache of what last worked: an invocation that times out, is
/// refused or answers unusably leaves every entry exactly as it was, because a failure is evidence
/// about one call and not about the server's tools.  There is no circuit breaker here in V1, so
/// nothing in this type changes after construction.
#[derive(Clone, Debug, Default)]
pub struct SessionExternalMcp {
    pub guard: Option<crate::database::tool_security::ExternalToolGuard>,
    servers: Arc<[ResolvedExternalMcp]>,
    /// LLM-visible name to the origin it stands for.  Routing resolves through the origin rather
    /// than through the name, so a Device MCP name can never reach an External MCP server and the
    /// original wire name is what a call is actually made with.
    routes: HashMap<String, ToolOrigin>,
    /// Where a server lives in `servers`, so an origin can find its handle without a scan.
    positions: HashMap<String, usize>,
}

impl SessionExternalMcp {
    pub fn new(servers: Vec<ResolvedExternalMcp>) -> Self {
        let mut routes = HashMap::new();
        let mut positions = HashMap::new();
        for (position, server) in servers.iter().enumerate() {
            positions.insert(server.server_key.clone(), position);
            for tool in server.tools.iter() {
                routes.insert(
                    tool.llm_name.clone(),
                    ToolOrigin::ExternalMcp {
                        server_key: server.server_key.clone(),
                        original_name: tool.original_name.clone(),
                    },
                );
            }
        }
        Self {
            guard: None,
            servers: Arc::from(servers),
            routes,
            positions,
        }
    }

    pub fn servers(&self) -> &[ResolvedExternalMcp] {
        &self.servers
    }

    /// The handle for one admitted server, by the immutable identity it was admitted under.
    ///
    /// The catalog never changes, so a server a session called once is still here afterwards; this
    /// is how a response that arrived too late is still attributed to the server that produced it.
    pub fn server(&self, server_key: &str) -> Option<&ResolvedExternalMcp> {
        self.positions
            .get(server_key)
            .map(|position| &self.servers[*position])
    }

    pub fn tool_count(&self) -> usize {
        self.routes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.routes.is_empty()
    }

    /// Resolves what a model named to the handle and the original wire name to call.  The
    /// LLM-visible name is only ever the key of this lookup: it is never the thing sent.
    pub fn find(&self, llm_name: &str) -> Option<(&ResolvedExternalMcp, &ResolvedExternalTool)> {
        let (server_key, original_name) = match self.routes.get(llm_name)? {
            ToolOrigin::ExternalMcp {
                server_key,
                original_name,
            } => (server_key, original_name),
            // A Device MCP name is never an External MCP route, whatever this catalog holds.
            ToolOrigin::Device { .. } => return None,
        };
        let server = &self.servers[*self.positions.get(server_key)?];
        let tool = server
            .tools
            .iter()
            .find(|tool| tool.original_name == *original_name)?;
        Some((server, tool))
    }
}

/// Application-owned resolution state.  The limiter is process-global by design: one server
/// saturating its bound must not be able to slow another server, Device MCP, or a session that
/// never touches that server at all.
pub struct ExternalMcpManager {
    http: reqwest::Client,
    limiter: Arc<ExternalMcpCallLimiter>,
    /// Process-owned, so every session and every server reports into the same counters.
    telemetry: Arc<dyn Telemetry>,
    per_server_timeout: Duration,
    overall_budget: Duration,
    limits: ExternalMcpLimitsConfig,
    network: crate::config::ExternalMcpNetworkConfig,
}

impl std::fmt::Debug for ExternalMcpManager {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ExternalMcpManager")
            .field("limiter", &self.limiter)
            .field("telemetry", &std::any::type_name::<Self>())
            .field("per_server_timeout", &self.per_server_timeout)
            .field("overall_budget", &self.overall_budget)
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

impl ExternalMcpManager {
    /// Builds the shared transport and the process-global call limiter.
    ///
    /// Redirects are disabled and TLS keeps its normal certificate chain and hostname checks: there
    /// is no insecure mode for a database record to ask for.
    pub fn new(config: &ExternalMcpConfig) -> Result<Self, reqwest::Error> {
        Self::new_with_telemetry(config, Arc::new(TracingTelemetry))
    }

    /// The same manager reporting into a caller-owned sink, so a deployment's metrics backend and a
    /// test's recorder are the same seam with a different sink.
    ///
    /// This and [`new`](Self::new) are deterministic test seams: they install a gate that stays
    /// open, so nothing refuses a permit. Production builds the manager through
    /// [`new_with_telemetry_and_gate`](Self::new_with_telemetry_and_gate) with the application's
    /// own gate, and that is the only constructor from which a running process should be built.
    pub fn new_with_telemetry(
        config: &ExternalMcpConfig,
        telemetry: Arc<dyn Telemetry>,
    ) -> Result<Self, reqwest::Error> {
        Self::new_with_telemetry_and_gate(config, telemetry, AdmissionGate::open())
    }

    /// The production seam: caller-owned telemetry and the application admission gate together, so
    /// the shared limiter refuses a new permit once the application has closed its gate and a
    /// Tool-round work item queued before shutdown cannot put a request on the network afterwards.
    pub fn new_with_telemetry_and_gate(
        config: &ExternalMcpConfig,
        telemetry: Arc<dyn Telemetry>,
        gate: Arc<AdmissionGate>,
    ) -> Result<Self, reqwest::Error> {
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(concat!("voice-agent-server/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self {
            http,
            limiter: Arc::new(ExternalMcpCallLimiter::with_gate(
                config.max_concurrent_calls_per_server,
                gate,
            )),
            telemetry,
            per_server_timeout: Duration::from_millis(config.per_server_resolution_timeout_ms),
            overall_budget: Duration::from_millis(config.overall_resolution_budget_ms),
            limits: config.limits.clone(),
            network: config.network.clone(),
        })
    }

    pub fn limiter(&self) -> &Arc<ExternalMcpCallLimiter> {
        &self.limiter
    }

    /// Resolves every enabled binding the Agent publishes into one fresh snapshot.
    ///
    /// Servers resolve in parallel under a per-server budget and an overall one, so a session is
    /// never held open by the slowest bound server.  Whatever survives is published; whatever did
    /// not is reported as a bounded exclusion and the Voice Session continues either way.
    pub async fn resolve_snapshot(
        &self,
        servers: &[AdmittedMcpServer],
        secrets: &dyn SecretResolver,
    ) -> ExternalMcpSnapshot {
        if servers.is_empty() {
            return ExternalMcpSnapshot::default();
        }
        // Two server keys that normalize to one namespace segment would make their tools
        // indistinguishable, so neither is published.  Resolving the whole group as unusable is
        // deterministic; picking a survivor by bind order would not be.
        let colliding = colliding_server_keys(servers);

        let resolutions = match tokio::time::timeout(
            self.overall_budget,
            join_all(
                servers
                    .iter()
                    .map(|server| self.resolve_one(server, secrets, &colliding)),
            ),
        )
        .await
        {
            Ok(resolutions) => resolutions,
            // The whole snapshot ran out of budget: every server loses its tools together rather
            // than leaving whichever finished first published as if the set were complete.
            Err(_) => servers
                .iter()
                .map(|server| {
                    Err(Excluded::for_server(
                        &server.key,
                        ExternalMcpExclusionReason::ToolsListTimeout,
                    ))
                })
                .collect(),
        };

        let mut published: Vec<ResolvedExternalMcp> = Vec::new();
        let mut exclusions: Vec<ExternalMcpExclusion> = Vec::new();
        for resolution in resolutions {
            match resolution {
                Ok(Resolved { server, elapsed }) => {
                    self.telemetry
                        .resolve_succeeded(&server.server_key, elapsed);
                    published.push(server);
                }
                Err(excluded) => {
                    self.telemetry.resolve_failed(
                        &excluded.server_key,
                        excluded.reason.as_str(),
                        excluded.elapsed,
                    );
                    exclusions.push(ExternalMcpExclusion {
                        server_key: excluded.server_key,
                        reason: excluded.reason,
                    });
                }
            }
        }

        // The aggregate cap is applied only once every server has passed on its own, and its
        // overflow drops the entire External MCP snapshot.  There is no partial catalog and no
        // "drop the last server" rule, because either would make the published set depend on bind
        // order and Device MCP would be the only thing left standing.
        let total = published
            .iter()
            .map(|server| server.tools.len())
            .sum::<usize>();
        // The aggregate cap is a fact about the whole snapshot, not about any one server's
        // resolution, so it gets its own counter and emits no per-server duration: every server in
        // here has already reported whether it resolved.
        if total > self.limits.max_tools_per_session {
            self.telemetry.session_tool_cap_exceeded();
            exclusions.extend(published.iter().map(|server| ExternalMcpExclusion {
                server_key: server.server_key.clone(),
                reason: ExternalMcpExclusionReason::SessionToolCapExceeded,
            }));
            published.clear();
        }
        for exclusion in &exclusions {
            tracing::warn!(
                event = "external_mcp_server_excluded",
                server_key = %exclusion.server_key,
                reason = exclusion.reason.as_str(),
                "An External MCP server contributed no tools to this admission"
            );
        }
        tracing::info!(
            event = "external_mcp_snapshot_resolved",
            server_count = published.len(),
            tool_count = total.min(self.limits.max_tools_per_session),
            excluded_count = exclusions.len(),
            "External MCP admission snapshot resolved"
        );
        ExternalMcpSnapshot {
            servers: published,
            exclusions,
        }
    }

    /// One server's whole journey: credential, handshake, paginated catalog, validation,
    /// publication.  Every step is bounded and none of them can reject the Voice Session.
    async fn resolve_one(
        &self,
        server: &AdmittedMcpServer,
        secrets: &dyn SecretResolver,
        colliding: &BTreeSet<String>,
    ) -> Result<Resolved, Excluded> {
        let started = std::time::Instant::now();
        match self.resolve_one_inner(server, secrets, colliding).await {
            Ok(server) => Ok(Resolved {
                server,
                elapsed: started.elapsed(),
            }),
            Err(excluded) => Err(Excluded {
                server_key: excluded.server_key,
                reason: excluded.reason,
                elapsed: started.elapsed(),
            }),
        }
    }

    async fn resolve_one_inner(
        &self,
        server: &AdmittedMcpServer,
        secrets: &dyn SecretResolver,
        colliding: &BTreeSet<String>,
    ) -> Result<ResolvedExternalMcp, Excluded> {
        let server_key = server.key.clone();
        if colliding.contains(&server_key) {
            return Err(Excluded::for_server(
                &server_key,
                ExternalMcpExclusionReason::ServerKeyCollision,
            ));
        }
        let reference = match &server.secret_ref {
            None => None,
            Some(raw) => match SecretRef::parse(raw.clone()) {
                // An unreadable reference is the same class as a missing one: the operator pointed
                // at something that cannot be read, and the reference itself never becomes a
                // diagnostic.
                Err(_) => {
                    return Err(Excluded::for_server(
                        &server_key,
                        ExternalMcpExclusionReason::SecretResolutionFailed("secret_invalid"),
                    ));
                }
                Ok(reference) => Some(reference),
            },
        };
        let mut client = match ExternalMcpClient::connect(
            &server_key,
            &server.url,
            &server.headers_json,
            &server.auth_type,
            server.auth_header_name.as_deref(),
            reference.as_ref(),
            Duration::from_millis(server.connect_timeout_ms.max(1) as u64),
            Duration::from_millis(server.request_timeout_ms.max(1) as u64),
            self.http.clone(),
            self.network.clone(),
            &self.limits,
            Arc::clone(&self.telemetry),
            secrets,
        ) {
            Ok(client) => client,
            Err(ConnectFailure::Secret(reason)) => {
                return Err(Excluded::for_server(
                    &server_key,
                    ExternalMcpExclusionReason::SecretResolutionFailed(reason),
                ));
            }
            Err(ConnectFailure::Unusable) => {
                return Err(Excluded::for_server(
                    &server_key,
                    ExternalMcpExclusionReason::ServerUnavailable,
                ));
            }
        };
        let call_timeout = client.call_timeout();
        let walk = async {
            client
                .initialize()
                .await
                .map_err(ExternalMcpExclusionReason::from)?;
            let catalog = self.walk_catalog(&client).await?;
            Ok::<_, ExternalMcpExclusionReason>(catalog)
        };
        let catalog = match tokio::time::timeout(self.per_server_timeout, walk).await {
            Ok(Ok(catalog)) => catalog,
            Ok(Err(reason)) => return Err(Excluded::for_server(&server_key, reason)),
            // The server did not finish its walk in its own budget.  Nothing discovered so far is
            // published: a partial catalog is exactly the stale-looking snapshot this avoids.
            Err(_) => {
                return Err(Excluded::for_server(
                    &server_key,
                    ExternalMcpExclusionReason::ToolsListTimeout,
                ));
            }
        };
        if catalog.tools.is_empty() {
            return Err(Excluded::for_server(
                &server_key,
                ExternalMcpExclusionReason::CatalogRejected,
            ));
        }
        Ok(ResolvedExternalMcp {
            server_key,
            namespace: catalog.namespace,
            client: Arc::new(client),
            tools: Arc::from(catalog.tools),
            call_timeout,
            limiter: Arc::clone(&self.limiter),
        })
    }

    /// `initialize` was accepted; now walk every page of the catalog and validate it whole.
    async fn walk_catalog(
        &self,
        client: &ExternalMcpClient,
    ) -> Result<PublishedCatalog, ExternalMcpExclusionReason> {
        let server_key = client.server_key().to_owned();
        let Some(namespace_segment) = normalize_external_tool_segment(&server_key) else {
            return Err(ExternalMcpExclusionReason::ServerUnavailable);
        };
        let namespace = format!("{EXTERNAL_NAMESPACE}.{namespace_segment}");

        let mut discovered: Vec<(String, String, Value)> = Vec::new();
        let mut pages = 0usize;
        let mut cursor: Option<String> = None;
        loop {
            pages += 1;
            if pages > self.limits.max_pages_per_server {
                return Err(ExternalMcpExclusionReason::CatalogRejected);
            }
            let page = client
                .list_tools_page(cursor.as_deref())
                .await
                .map_err(ExternalMcpExclusionReason::from)?;
            if discovered.len().saturating_add(page.tools.len()) > self.limits.max_tools_per_server
            {
                return Err(ExternalMcpExclusionReason::CatalogRejected);
            }
            for tool in page.tools {
                // Order matters and follows the untrusted document inward: the raw page bytes were
                // already capped before it was parsed, and now each tool's canonical schema and
                // description are capped before anything is inspected for meaning.
                if tool.description.len() > self.limits.max_tool_description_bytes
                    || serde_json::to_vec(&tool.input_schema)
                        .is_ok_and(|raw| raw.len() > self.limits.max_tool_schema_bytes)
                    || validate_external_tool_schema(&tool.input_schema).is_err()
                {
                    return Err(ExternalMcpExclusionReason::CatalogRejected);
                }
                discovered.push((tool.name, tool.description, tool.input_schema));
            }
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        let catalog = ExternalToolCatalog::publish(&namespace, discovered)
            .map_err(|error| ExternalMcpExclusionReason::from(ExternalMcpError::from(error)))?;
        if catalog.dropped() > 0 {
            // A name the model could never have called is not an error, but it is never silent
            // either: the server is not offering what it appears to offer.
            tracing::warn!(
                event = "external_mcp_server_excluded",
                server_key = %client.server_key(),
                reason = "mcp_tool_name_invalid",
                dropped_tool_count = catalog.dropped(),
                "An External MCP server announced tools no LLM-visible name could represent"
            );
        }
        Ok(PublishedCatalog {
            namespace,
            tools: catalog.tools().to_vec(),
        })
    }
}

/// A server that published tools, with how long producing them took.
struct Resolved {
    server: ResolvedExternalMcp,
    elapsed: Duration,
}

/// The outcome of one server's resolution: the tools to publish, or the bounded reason not to.
struct Excluded {
    server_key: String,
    reason: ExternalMcpExclusionReason,
    elapsed: Duration,
}

impl Excluded {
    fn for_server(server_key: &str, reason: ExternalMcpExclusionReason) -> Self {
        Self {
            server_key: server_key.to_owned(),
            reason,
            elapsed: Duration::ZERO,
        }
    }
}

struct PublishedCatalog {
    namespace: String,
    tools: Vec<ResolvedExternalTool>,
}

/// Server keys that share one normalized namespace segment.  Grouping is ordered by key, so the
/// answer never depends on the order the bindings were created in.
fn colliding_server_keys(servers: &[AdmittedMcpServer]) -> BTreeSet<String> {
    let mut namespaces: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for server in servers {
        if let Some(segment) = normalize_external_tool_segment(&server.key) {
            namespaces
                .entry(segment)
                .or_default()
                .insert(server.key.clone());
        }
    }
    namespaces
        .into_values()
        .filter(|keys| keys.len() > 1)
        .flatten()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_wire_failure_maps_to_a_bounded_reason() {
        let classes = [
            (
                ExternalMcpError::ServerUnavailable,
                ExternalMcpExclusionReason::ServerUnavailable,
            ),
            (
                ExternalMcpError::InitializeFailed,
                ExternalMcpExclusionReason::InitializeFailed,
            ),
            (
                ExternalMcpError::InitializeAuthFailed,
                ExternalMcpExclusionReason::InitializeFailed,
            ),
            (
                ExternalMcpError::InitializeProtocolError,
                ExternalMcpExclusionReason::InitializeFailed,
            ),
            (
                ExternalMcpError::InitializeTimeout,
                ExternalMcpExclusionReason::ToolsListTimeout,
            ),
            (
                ExternalMcpError::ToolsListTimeout,
                ExternalMcpExclusionReason::ToolsListTimeout,
            ),
            (
                ExternalMcpError::ToolsListInvalid,
                ExternalMcpExclusionReason::ToolsListInvalid,
            ),
            (
                ExternalMcpError::ToolNameInvalid,
                ExternalMcpExclusionReason::ToolsListInvalid,
            ),
            (
                ExternalMcpError::ToolNameCollision,
                ExternalMcpExclusionReason::ToolNameCollision,
            ),
        ];
        for (wire, reason) in classes {
            assert_eq!(ExternalMcpExclusionReason::from(wire), reason);
        }
    }
}
