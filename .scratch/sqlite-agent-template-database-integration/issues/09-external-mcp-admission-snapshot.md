# 09: External MCP admission snapshot

**What to build:** A newly admitted Agent session receives a fresh, bounded and fail-soft External MCP tool snapshot that is safe to expose to the LLM and immutable for session lifetime.

**Blocked by:** 04: External MCP desired configuration; 07: Provider Load Plan và Effective Session Profile.

**Status:** resolved

- [x] Admission resolves Agent MCP bindings with per-server and total bounded budgets, resolves credential once into ExternalMcpClient, runs initialize/tools-list pagination and excludes optional failing servers without rejecting Voice.
- [x] Discovery validates raw catalog caps, schema subset, normalized namespaces, collisions and aggregate tool caps before LLM conversion; no stale tools/list catalog is authoritative.
- [x] ResolvedExternalMcp stores client handle, original tool mapping, timeout and bounded metadata in Effective Session Profile; SessionActor does not own raw SecretValue.
- [x] Global per-server call limiter exists with configured bound; tool catalog remains immutable after timeout/auth/unavailable/invalid response and V1 has no circuit breaker.
- [x] Admission tests prove fail-soft discovery, secret-resolution failure redaction, TLS/network/redirect policy, schema/name/result caps, limiter isolation and immutable Session Tool Catalog.

## Answer

`src/tools/external_mcp/` is the whole External MCP boundary: `registry` (pure namespace
normalization, bounded supported-schema validation, per-server catalog), `transport`
(process-global per-server call limiter, fixed request assembly, destination policy dial), `client`
(the credential-owning handle) and `manager` (application-owned admission resolution).
`database/external_mcp.rs` is the single bounded read of the Agent's enabled bindings.

Admission resolves the profile first, then the snapshot. `AppState` owns one `ExternalMcpManager`,
so the transport, the connection pool and the `ExternalMcpCallLimiter` are process-global while the
tools are per admission. Every admission re-walks `initialize` and the whole paginated `tools/list`;
a previous session's verification is never authority. `ResolvedExternalMcp` carries
`Arc<ExternalMcpClient>`, which is where the `SecretValue` lives — `SessionActor` holds the snapshot
and nothing that can print a credential. `into_active_profile` became `into_admitted_profile` so the
profile, the switch catalog and the External MCP tools install as one value and cannot be split.

Discovery order is the guide's: page body cap, per-tool description/schema caps, structural schema
validation, name normalization and collision, then publication. `ExternalToolCatalog::publish` drops
a name with no representable segment and refuses the whole server on a collision; two server keys
that normalize alike are both refused rather than resolved by bind order. The aggregate
`max_tools_per_session` is applied only after every server passes, and its overflow drops the entire
External MCP snapshot.

Fail-soft is total: a secret that does not resolve, a destination outside the allowlist, a redirect,
a TLS failure, a malformed page, a cap breach and a server-key collision each exclude only that
server, and the WebSocket still upgrades. No `tools/call` outcome changes the catalog, and V1 has no
circuit breaker.

Three findings worth recording:

- A `tools/call` response is read under the operator's result cap plus a bounded envelope allowance;
  the canonical result is then checked against the cap on its own. Using the result cap directly as
  the HTTP body cap refused legal results by their own JSON-RPC framing.
- A configuration whose derived page budget cannot be buffered is rejected at config load rather
  than surprising an operator at admission time.

## Loopback, formalized

Loopback is inside the LAN scope `allow_http_lan` opens, because a homelab MCP server usually runs
on the same host. It grants nothing on its own, and ADR-0056 now says so in terms: an HTTP loopback
destination is valid only when the exception is on **and** the operator named that destination —
an IP literal in `allowed_cidrs`, a hostname in `allowed_hosts` — and the same destination still
passes the post-DNS check. An empty allowlist refuses loopback too, and there is no test-only
bypass.

Making that true for IPv6 required a fix: `Url::host_str` renders an IPv6 literal bracketed, so
`[::1]` matched no CIDR an operator had written and silently took a weaker path. Host matching now
uses the typed `url::Host::{Domain, Ipv4, Ipv6}`.

## Bounded telemetry

`src/telemetry.rs` is the process-owned seam. A caller names a metric from a fixed set and a label
from a bounded class, and the only free-form value it can pass is a `server_key` — which the Admin
API already bounds to 64 bytes of `[a-z0-9_]`. A destination, a header value, a credential, a
protocol session id, a tool argument and a tool result are not parameters of the seam at all.

Reported: `mcp_resolve_success_total`, `mcp_resolve_failure_total{reason}`,
`mcp_resolve_duration_ms`, `external_mcp_tool_calls_total{server_key,outcome}`,
`external_mcp_tool_call_duration_ms{server_key,outcome}` and
`external_mcp_call_limiter_rejected_total`. `ExternalMcpManager::new` wires the default
`TracingTelemetry` sink, which renders each observation as a structured event carrying the metric
name and the bounded classes — enough to alert on and for a collector to scrape, without this
service choosing a metrics backend the deployment has not asked for. `new_with_telemetry` takes any
sink, and `RecordingTelemetry` lets a test assert the label contract rather than take it on trust.

Two decisions the guide left open:

- `mcp_resolve_duration_ms` carries the same `server_key` and `outcome` as the counter it sits
  beside. A duration with no label cannot be attributed to a server at all.
- The aggregate cap `external_mcp_session_tool_cap_exceeded` is a fact about the whole snapshot, so
  it is not counted as a server resolve failure. A server that resolved and published still counts
  as resolved; counting it twice with a duration it never had would be worse than not counting it.

## Review follow-up

Code review (Standards and Spec axes) findings that were fixed rather than noted:

- The per-server limiter waited not at all, so a momentarily busy server read as unavailable.
  `ExternalMcpCallLimiter::acquire_within` now waits for the caller's own remaining turn budget, and
  `call_tool` takes that budget: it bounds the wait *and* the request, so a turn that runs out
  sends nothing at all. This is ADR-0053's "chờ trong turn budget" without the executor that owns
  the budget yet.
- `structuredContent` short-circuited result validation, so a result carrying an unsupported content
  item alongside it was accepted. Every content item is now validated whatever else the document
  carries.
- `notifications/initialized` accepted any non-2xx as success. A notification is not a request, so
  its answer is judged by status alone now.
- `mcp_tool_name_invalid` and `secret_missing` were unreachable classes. A name with no
  representable segment is counted and reported as a bounded diagnostic, and a typed auth with no
  reference is reported as `secret_missing`.
- `ToolOrigin` was dead: routing used raw indices. `SessionExternalMcp` now routes
  `llm_visible_name → ToolOrigin → server + original wire name`, so a Device MCP name cannot
  resolve to an External MCP server by construction.
- The `Default` impls for `[mcp.external]` restate the serde defaults on purpose — otherwise
  `McpConfig::default()` would be a configuration that fails its own validation. A test now pins
  the two spellings together, and `docs/04-configuration.md` documents the section and its rules.

Judgement calls the review raised that were deliberately kept, and why:

- The External MCP tool advertisement and execution stay with ticket 10's Tool-round Executor, so
  no tool is ever advertised that the session cannot serve. The snapshot is installed in
  `SessionActor` and is provably immutable, and three new actor tests assert what it holds and that
  it renders without a credential.
- A database read that fails while resolving the optional MCP bindings stays fail-soft. ADR-0053
  scopes fail-soft to optional External MCP, and refusing a Voice Session over a binding read would
  contradict the ticket's own headline. Readiness degradation is ticket 12.
- (Superseded: the seam below was added rather than deferred. The reason classes that were on the
  wire and in the events are now counters too.)

Validation: `cargo fmt --check`, `cargo clippy -p voice-agent-server --all-targets` back at the
pre-change warning count (12 lib / 13 lib test), and `cargo test --workspace` fully green across 47
test binaries, including the three `speechoutput_tracer` timing tests that earlier tickets noted as
flaky.

The telemetry test resolves a real snapshot against a real MCP server carrying a bearer credential
and a static header, then reads back every recorded label. It asserts the label set is closed
(`server_key`, `reason`, `outcome` and nothing else), that a `server_key` is the Admin API's bounded
resource key, and that the recording contains none of the secret value, the secret reference, the
static header value, the wire tool name, the URL, the loopback address, or the argument the model
sent.
