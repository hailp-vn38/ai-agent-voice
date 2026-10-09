# Provider Live Testing & External MCP Connection / Tool Discovery — Implementation Guide

> **Scope:** Rust `voice-agent-server` + Vue `admin-web`; design + implementation instructions, **no production code changes in this document**.  
> **Repository:** https://github.com/hailp-vn38/ai-agent-voice  
> **Source reviewed:** `main` @ `37641bc0a63428a038904c2acd40ada949db88bf` (2026-10-09).  
> **Primary goal:** Test unsaved Provider/MCP configuration from Admin Web, run real Provider inference, probe MCP connectivity, discover MCP tools, and offer coherent Create/Detail experiences without modifying Agent runtime or tool permissions.

## 0. Decisions and non-goals

1. **One real execution path per operation.** Reuse Provider factories, managed runtime lifecycle, diagnostics, and the existing RMCP transport/catalog logic. New handlers do not independently instantiate ONNX or build a second MCP protocol client.
2. **Draft is a first-class *source* of an invocation, not a database row.** Testing MUST NOT create, patch, auto-enable, or bind Provider/MCP resources. No fake DB id or deployment id.
3. **Two categories of tests:** Provider inference (ASR/LLM/TTS); MCP `initialize` (connection) and full paginated `tools/list` (discovery). MCP tests MUST NOT issue `tools/call`.
4. **Read-only testing regarding persistent authority:** Tests cannot mutate Agent bindings, tool approvals/fingerprints, Template bindings, desired config, runtime desired revision, or existing Voice Session snapshot. Request-local results may be displayed in the UI; ordinary bounded privacy-safe operational metrics are permitted.
5. **Credentials follow the current ADR 0083.** Provider/MCP Admin CRUD accepts optional **write-only top-level `api_key`**; stored credentials are AES-256-GCM encrypted (`credential_json`) and take priority over environment fallback. Never re-introduce the obsolete Admin `secret_ref` input. Draft tests may accept ephemeral `api_key` as a write-only request value, **without sealing or persisting it**. Read APIs return only masked credential metadata. Existing saved-resource credentials are resolved server-side only.
6. **MCP connectivity is an observed, point-in-time test**, not a persistent `connected` flag. Empty valid tool catalogs may pass connection while discovery reports `0` tools. Discovery does not approve tools. A `failed` MCP discovery does not mean the saved server is automatically disabled.
7. **HTTP/HTTPS outbound policy must reflect the current ADR 0056**, which permits HTTP/LAN/loopback and uses optional `allowed_hosts`, with redirects off and standard HTTPS certificate checks. Tightening this policy is a separate ADR/project decision, not a silent behavior change in this feature.
8. Include saved-provider diagnostics compatibility, VAD unchanged unless deliberately generalized later. Speaker/Vision, MCP resource read/write, and Device MCP are outside this task except where shared code/tests must be preserved.

## 1. Verified state of `main`

### 1.1 Recent changes that supersede the previous design

- `37641bc` + `eab0728`: SQLx access moved into database domain modules. New application logic SHOULD call database interfaces and **not** insert SQLx into `app/admin` or `tools/external_mcp`.
- `30b69e3`: Admin-managed encrypted credentials for Provider and MCP. Current accepted [ADR 0083](docs/adr/0083-admin-managed-resource-credentials.md) replaces older deployment-only assumptions.
- `3637569`: MCP page proxy handling fixed. Preserve existing `/mcp` route.
- `c252f79`: updated MCP approval and Provider readiness test fixtures; keep authorization and read-only readiness contracts intact.

### 1.2 Current code and gaps

| Location | Existing behavior | Work required |
|---|---|---|
| `crates/voice-agent-server/src/app/admin/provider_tests.rs` | Real saved-provider `POST /providers/{key}/test/{asr,llm,tts,vad}`, ASR WAV parse, LLM JSON, TTS WAV | Add draft HTTP adapters, share codecs/errors/diagnostic path. |
| `src/services/provider_diagnostic.rs` | Saved-provider DB snapshot → managed runtime; admission, timeout/cancel/terminal ack/quarantine | Accept caller-provided validated draft target/lease; do not synthesize a loaded DB revision. |
| `src/services/provider_runtime/identity.rs`, `registry.rs` | `ProviderIdentity::{Database,Deployment}`, acquire requires DB id > 0; deployment is id 0 | Add explicit ephemeral identity + acquisition while preserving accounting, resource-key isolation, admission and teardown. |
| `src/services/provider_runtime/factory/mod.rs`, `src/providers/database_loader.rs` | Factory/materialization behavior branches on `row.id == 0`; credentials resolved via `SecretResolver` | Replace id-based source assumption at seam before passing drafts; reuse typed validation and secret handling. |
| `src/database/provider_config.rs` | Canonical non-secret adapter config, bounded, reject credential fields | Reuse exactly this validator for draft; do not add `api_key` to `config_json`. |
| `src/database/providers.rs` | Provider reads/writes and desired snapshots in database module | New saved lookups, when necessary, belong here. Draft execution has no SQL write. |
| `src/app/admin/providers.rs` | Provider CRUD with top-level `api_key` encryption and masked response | Keep unchanged contract and optimistic revision semantics. |
| `apps/admin-web/src/components/providers/ProviderCreateDrawer.vue` | 3-step create; step 3 only review/save | Replace step 3 with Test & Review, no save required to test. |
| `apps/admin-web/src/components/admin/ProviderDetailModal.vue` | `runTest()` returns pass/fail based on `status === 'ready'` | Delete fake check; integrate real shared test panel. |
| `apps/admin-web/src/components/admin/ProviderFormModal.vue`, `src/stores/admin.ts` | Flattened model/description/endpoint edit may replace typed adapter config with incomplete JSON | Preserve full canonical config; share schema-driven editor. |
| `src/app/admin/mcp_servers.rs`, `src/database/mcp_servers.rs` | MCP CRUD, encrypted credential metadata, Agent binding CRUD; no probing routes | Add saved MCP snapshot read method in database module, new test handlers separate from mutations. |
| `src/tools/external_mcp/client.rs` | Validated `ExternalMcpClient::connect`, RMCP `initialize`, `list_tools_page`, guarded `tools/call` | Reuse connect/init/list for probes; never call guarded tool execution. |
| `src/tools/external_mcp/manager.rs` | Per-Voice-Session `resolve_snapshot`, complete paginated catalog validation, rejects unusable catalogs | Extract reusable safe probe/catalog-walk implementation; preserve session admission behavior. |
| `src/tools/external_mcp/registry.rs` | Tool name normalization, schema subset validation, published `external.<server_key>.<tool>` | Use same normalization/schema rules for discovery response; make dropped/rejected tools explicit. |
| `apps/admin-web/src/views/McpServersView.vue` | MCP list/filter/create/edit/enable/delete; cards do not open detail | Add navigable card and detail page, retain existing list layout styles. |
| `apps/admin-web/src/components/mcp/McpServerFormModal.vue` | Saved create/edit accepts write-only key but cannot test before saving | Add Connect / Discover test area and reset stale results when draft changes. |
| `apps/admin-web/src/api/mcp.ts`, `src/api/types/mcp.ts` | CRUD only | Add typed probe/discovery calls and response types. |
| `apps/admin-web/src/router/index.ts` | `/mcp` only | Register `/mcp/:key` while keeping `/mcp`. |

**Important:** `providersApi.test*` methods exist, but current Provider Detail does NOT call them. MCP `/provider-adapters/{adapter}/capabilities/discover` is **Provider model/voice capability discovery**, not External MCP `tools/list`; do not conflate the two.

## 2. Shared architecture and seams

```text
                       Admin Web
            ┌─────────────┴──────────────┐
       Provider Test UI              MCP Test UI
       Create / Detail               Create / Detail
            │                             │
     Admin authenticated HTTP routes (bounded request parsers)
            │                             │
     ProviderTestRunner              McpDiagnosticRunner
      ├─ source resolve              ├─ source resolve
      ├─ typed config validate       ├─ URL/auth/network validate
      ├─ runtime acquire             ├─ RMCP initialize
      ├─ existing diagnostic run     └─ full tools/list (optional)
      └─ lease/ack/quarantine               │
            │                          ExternalMcpClient
       ProviderRuntimeManager        shared catalog walk
            │                             │
    ProviderRegistry/Factories   ExternalToolCatalog/Schema
            └─────────────┬──────────────┘
                   sanitized outputs
```

### 2.1 Provider runner external interface

```rust
// Design sketch. Final types should live near existing diagnostics, not in HTTP handlers.
pub enum ProviderTestSource {
    Saved { key: String },
    Draft {
        provider_type: ProviderType,
        adapter: String,
        config: serde_json::Value,
        credential: DraftCredentialSource,
    },
}

pub enum DraftCredentialSource {
    None,
    Inline(SecretValue),
    // For testing UNSAVED edits without revealing stored credential:
    SavedVersion { key: String, expected_revision: i64 },
}

pub enum ProviderTestInput {
    Asr { audio: PcmF32Mono },
    Llm { text: String },
    Tts { text: String, voice: Option<String>, language: Option<String> },
}

pub async fn run_test(
    &self,
    source: ProviderTestSource,
    input: ProviderTestInput,
) -> Result<ProviderTestResult, ProviderTestError>;
```

`ProviderTestRunner` owns source validation/credential resolution/runtime acquisition, and calls the existing `ProviderDiagnosticService` execution/acknowledgement logic. No other caller handles native thread cleanup. Test outputs report a `test_source: "draft"|"saved"`, not a fictitious `tested_provider_id` on draft. `loaded` in a draft means the test lease became ready, **not** that a persisted Provider is Ready for Agent use.

### 2.2 Draft runtime identity: explicit source, not id-zero trick

- Introduce `ProviderIdentity::Draft(Uuid)` or a similarly *typed* identity at `services/provider_runtime/identity.rs`; add a single narrowly scoped `acquire_draft_until(...)` method sharing the *private* `acquire_version_until` implementation.
- Refactor the materialization input to carry `ProviderSource::{Database, Deployment, Draft}` at the relevant seam; current `row.id == 0` is specifically deployment and must not be reused for draft. Avoid a long-lived polymorphic hierarchy unless a second implementation truly varies.
- Draft spec must be canonical validated, adapter-kind matched, and derived from compiled registry. Resource sizing/allowed local adapter/model/native execution settings use **exact same** deployment-owned budgets as saved providers. Return controlled unsupported/unavailable errors when an adapter isn't configured or assets cannot be prepared.
- Drafts get unique logical quota identity and a lifetime covering the test. Share native backing resource ONLY where verified ResourceKey and credential-isolation rules permit. Do not share authenticated remote clients across credentials or hash secrets into resource IDs/metrics.
- The manager owns loading reservations even when HTTP cancellation occurs. A UI request timeout does NOT permit premature drop of active native work. Releasing draft lease can leave a bounded resident cache entry under existing TTL/pressure eviction; never retain draft as an unbounded per-request resource.
- Preserve singleflight for identical resident physical resources, pilot admission gate, max-concurrent diagnostic semaphore, global workload quotas, shutdown lifecycle, and terminal ack/quarantine.
- Keep metadata-only GET/readiness. Draft test must never mutate desired provider version, template bindings, Agent session profile or runtime status response.

### 2.3 Credentials: three mutually exclusive modes

| Context | Credential behavior |
|---|---|
| Saved test by `{key}` | Resolve saved encrypted record server-side, else existing deployment-environment fallback; never expose plaintext. |
| New draft | Optional write-only `api_key` inside request (separate from config); keep in `SecretValue`/zeroizing allocation; **do not call `seal()`** and do not write database. |
| Unsaved edit of saved resource | Either new inline `api_key`, OR `saved_credential: { key, expected_revision }` if compatible auth/adapter; server resolves existing credential after checking revision. Reject mixing both. |

Never send encrypted record, credential UUID/nonce, secret reference, or raw key back to web. No key in URL, query, headers other than standard Admin bearer or logging. Existing `SecretResolver` has `resolve()`/`seal()`; introduce a small request-scoped resolver/credential value adapter **internal to the runner** to avoid round-tripping ephemeral plaintext through ciphertext/SQLite. Stored-resource snapshot resolution remains in database/read seam. For MCP saved-credential reuse, reject switching auth type/header without a compatible inline credential.

**Browser security:** Existing Admin Web already blocks credential submission over unsafe non-loopback HTTP. Reuse that guard for BOTH provider/MCP draft tests, and scrub/clear key text when closing modal or switching target; do not persist draft keys to localStorage, Pinia persistence, URL or browser logs. Do not automatically echo plaintext credentials in "view request JSON".

## 3. Provider HTTP contract

Use `/api/admin` prefix. Keep existing saved routes **unchanged**:

- `POST /providers/{key}/test/llm` — current body `{ "input": "..." }`, JSON result.
- `POST /providers/{key}/test/tts` — current body `{ "text": "...", "voice"?: "...", "language"?: "..." }`, `audio/wav` with timing/runtime headers.
- `POST /providers/{key}/test/asr` — raw `audio/wav`, JSON transcript.
- `POST /providers/{key}/test/vad` — unchanged.
- `POST /providers/{key}/prepare` — unchanged.

Add only **three** draft inference routes initially:

| Route | Request | Success |
|---|---|---|
| `POST /provider-tests/llm` | JSON `{provider:{type,adapter,config_json,api_key?|saved_credential?},input:{text}}` | JSON `result.text`, `metrics.elapsed_ms`, `test_source:"draft"` |
| `POST /provider-tests/tts` | JSON `{provider:...,input:{text,voice?,language?}}` | `audio/wav` and bounded headers for elapsed & source |
| `POST /provider-tests/asr` | `multipart/form-data`: `provider` JSON part, `audio` WAV binary part | JSON `result:{text,language}`, duration, elapsed, RTF |

Examples (dummy credentials only):

```json
{
  "provider": {
    "type": "llm",
    "adapter": "openai",
    "config_json": {"base_url": "https://api.example.com/v1", "model": "demo-model", "timeout_ms": 30000},
    "api_key": "<ephemeral-key-not-to-store>"
  },
  "input": {"text": "Xin chào, bạn là ai?"}
}
```

```json
{
  "test_source": "draft",
  "type": "llm",
  "status": "success",
  "result": {"text": "..."},
  "metrics": {"elapsed_ms": 742},
  "runtime": {"test_runtime_ready": true, "persisted_runtime_modified": false}
}
```

Draft response is **not** a Provider create response: omit DB id, revision, production `runtime_matches_desired`, `requires_restart`, or `provider_key` unless explicitly nullable and clearly defined. Return request ID via the existing Admin middleware. Preserve existing saved response shape for compatibility.

**ASR media:** Browser mic normally records at the device's sample rate; explicitly resample to the adapter's `capabilities.input_sample_rates`, construct mono PCM16 WAV (initial supported ASR adapters announce 16 kHz), and submit as file. Reuse `parse_asr_wav` and its pre-existing 30-second / 5 MiB bounds. Do not silently send WebM/Opus as `audio/wav`. Reject malformed WAV, 0 Hz, unsupported rate/channel/sample type, oversized/ambiguous multipart, and excessive duration before heavy runtime acquire. Implement a streaming/bounded multipart parser; do not read arbitrary-size parts. Bound JSON config per existing `MAX_PROVIDER_CONFIG_BYTES` and total parts independently.

**LLM:** A single tool-free one-turn request (`LlmRequest::text_turn`), not an Agent prompt or MCP-enabled session. Reuse current input cap (8 KiB), diagnostic output bound (currently 32 KiB in path), model timeout + global test timeout.

**TTS:** Reuse current 4 KiB text bound and validated voice/language selection. Return actual playable WAV, not PCM mislabeled as WAV. Use `URL.createObjectURL()` and revoke it on new result, modal close, or unmount. Do not permanently write audio.

**HTTP errors:** Reuse centralized error schema `{error:{code,request_id}}` / actual existing `error()` shape; distinguish `invalid_test_input`, `provider_config_invalid`, `provider_type_mismatch`, `provider_runtime_busy`, `provider_unavailable`, `provider_test_timeout`, `provider_test_failed`, `credential_missing/invalid`, `credential_storage_unavailable`, `revision_conflict` as applicable. Never embed upstream body, URL, token, prompt/transcript, raw audio or sensitive config in errors/logs. Status guidance: 400 malformed/validation; 409 revision/unsupported production resource readiness; 413 payload too large; 429 admission busy; 502 malformed/failed upstream; 503 unavailable; 504 timeout. Preserve exact existing codes for legacy routes.

## 4. MCP diagnostics architecture

### 4.1 A deep `McpDiagnosticRunner` module

Introduce `crates/voice-agent-server/src/tools/external_mcp/diagnostic.rs` (or an adjacent module under `services` if dependency direction requires it). Give Admin handlers a compact interface:

```rust
pub enum McpProbeSource {
    Saved { key: String },
    Draft {
        config: McpProbeConfig,
        credential: DraftCredentialSource,
    },
}

pub enum McpProbeOperation { Connect, Discover }

pub async fn probe(
    &self,
    source: McpProbeSource,
    operation: McpProbeOperation,
) -> Result<McpProbeResult, McpProbeError>;
```

For saved probes, read one **bounded** canonical `McpServerRow` snapshot via `database/mcp_servers.rs` using `mcp_by`/a new purpose-built read method, then derive its credential reference using existing `credentials::reference()`. For draft probes, validate identical URL/auth/timeouts to Admin create (`valid_desired_url`, `McpAuth`, allowed header name) and use inline/saved-compatible request-scoped credentials. No database write in either path. An Admin can explicitly test a disabled saved MCP without enabling it; clearly label this as a manual probe rather than production availability.

Do not call `ExternalMcpManager::resolve_snapshot` as an opaque Agent operation, because it filters enabled Agent bindings, applies aggregate **session** caps and conflates an empty catalog with a failed connection. Extract **internal reusable functions** from its existing `resolve_one_inner` + `walk_catalog`, preserving parsing, source validation, timeout, schema, normalization and RMCP semantics. Production session admission must still apply its original fail-soft/whole-catalog policy.

**Connect operation:** `ExternalMcpClient::connect()` → `initialize()` including RMCP `initialized` notification → return success/failure + elapsed. Connection success does **not** imply tools exist. Do not start SSE receive loops beyond RMCP's lifecycle requirements; no legacy transport fallback.

**Discover operation:** Same initialize → paginated `list_tools_page()` until no cursor → validate complete catalog with existing bounds, name normalization & `ExternalToolCatalog::publish()`. Return only complete usable catalog with original name, LLM-visible published name, description and bounded input schema. If a page fails, times out, breaches a cap, or schema is unsupported, return failure; **no partial catalog is shown as success**. Include bounded `dropped_tools` count only if normalization legitimately dropped names under current publication policy; name collisions/rejected schema stay rejected as in production.

Probe run MUST NEVER call `call_tool_guarded()`/`tools/call`, alter live Agent approvals, insert authoritative observed contracts into database, or silently link MCP to an Agent. Any later feature to promote a probe into a reviewable observed contract needs separate authorization/observation design; a manual Admin discovery result is not automatically trusted Agent authority.

**Limits:** Share lifecycle `AdmissionGate` and existing RMCP `reqwest` client/network/TLS/redirect restrictions, `per_server_resolution_timeout_ms`, `overall_resolution_budget_ms` where appropriate, connection timeout, request timeout, per-server pages/tools/schema/description/read-byte caps. Add small process-owned Admin probe semaphore (`api.mcp_tests.max_concurrency`, default proposal 2) and bounded total attempt deadline (`api.mcp_tests.timeout_ms`, proposal 30s, capped to configured RMCP budgets); derive actual production defaults after inspection and validate config in `config/validation.rs`. Never bypass a lower existing cap by adding a larger Admin timeout. Aborting HTTP should not leave unbounded remote operations or retained session references.

Network-policy warning: Current ADR 0056 explicitly allows HTTP to LAN/loopback (potential plaintext auth) and optional `allowed_hosts`, without CIDR blocking. Show a risk warning when credential-bearing MCP endpoint uses non-TLS HTTP; **do not override** currently accepted policy. Keep auth material and destinations out of audit/metrics labels and errors. Rate-limit expensive probes so an authenticated but mistaken operator cannot flood arbitrary reachable servers.

### 4.2 MCP routes: saved and draft

| Route | Body | Result |
|---|---|---|
| `POST /mcp-tests/connection` | Draft MCP config + optional ephemeral key | JSON connection result; no row required |
| `POST /mcp-tests/discover` | Same draft config | JSON full catalog result; no row required |
| `POST /mcp-servers/{key}/test/connection` | `{}` | JSON using saved revision/credential |
| `POST /mcp-servers/{key}/test/discover` | `{}` | JSON using saved revision/credential |

Draft payload (valid API examples use fake key):

```json
{
  "server": {
    "key": "weather",
    "url": "https://mcp.example.com/mcp",
    "auth": {"type": "bearer"},
    "api_key": "<ephemeral-token>",
    "connect_timeout_ms": 5000,
    "request_timeout_ms": 30000
  }
}
```

For edit-unsaved values, allow `server.saved_credential = {"key":"weather","expected_revision":7}` **instead of** `api_key`. This sources only the credential from the saved row; URL and auth/timeouts are taken from the draft request after validation. Require compatible auth type/header; reject mismatches and simultaneous inline + saved credential; revision check happens before remote traffic. A completely saved test can use saved routes instead.

Connection response:

```json
{
  "test_source": "saved",
  "operation": "connection",
  "status": "success",
  "protocol": "streamable_http",
  "handshake": "initialized",
  "metrics": {"elapsed_ms": 178},
  "saved_revision": 7
}
```

Discovery response (illustrative; no approval semantics):

```json
{
  "test_source": "saved",
  "operation": "discover",
  "status": "success",
  "metrics": {"elapsed_ms": 301},
  "catalog": {
    "count": 2,
    "dropped_count": 0,
    "complete": true,
    "tools": [
      {
        "name": "get_weather",
        "llm_name": "external.weather.get_weather",
        "description": "Look up forecast data",
        "input_schema": {"type": "object", "properties": {"city": {"type": "string"}}}
      },
      {
        "name": "get_alerts",
        "llm_name": "external.weather.get_alerts",
        "description": "List alerts",
        "input_schema": {"type": "object", "properties": {}}
      }
    ]
  }
}
```

Implement via safe serializable response DTOs rather than leaking `ResolvedExternalMcp` handles, `ExternalMcpClient` internals or raw `rmcp` wire objects. Return sanitized stable reason codes (`mcp_connect_timeout`, `mcp_auth_failed`, `mcp_initialize_failed`, `mcp_tools_list_timeout`, `mcp_catalog_rejected`, `mcp_rate_limited`, `mcp_credential_missing`, `revision_conflict`, etc.) and `request_id`. Do not send server error strings or raw remote tool-result content.

### 4.3 Distinguish observation, discovery, and permission

- **Observed in probe:** a one-time inspection performed explicitly by Admin; safe to show in the current browser state. It says nothing about tool permission.
- **Published into Voice Session:** production only, after complete validated server resolution at admission. Same namespace/catalog publication rules as probe.
- **Approved for Agent:** separately reviewed via existing Agent-scoped External MCP policy (`/agents/{key}/tool-allowlist` and `external-tools.ts`); a tool listed in MCP Detail is NOT automatically granted.
- On saved MCP URL/auth/credential/config mutation, existing security invalidation must remain. Manual probe MUST NOT clear, approve or re-stamp stale reviews. If an MCP becomes disabled/deleted, existing revocation semantics remain.
- Do not store ephemeral discovery schema in database by default; the operator can inspect it immediately. Optional persistence for an Admin history/cached catalog is a **separate** follow-up with freshness, retention and authorization contracts.

## 5. Admin Web UX design

### 5.1 Shared Provider Test Panel

Add a single `apps/admin-web/src/components/providers/ProviderTestPanel.vue` rendered in:

- `ProviderCreateDrawer.vue` step 3 "Test & Review" (config/credential draft, no save prerequisite).
- `ProviderDetailModal.vue` test section (call saved endpoint).
- `ProviderFormModal.vue` editing case (call draft endpoint with complete *unsaved* config and optional inline key / saved-credential revision).

The shared panel receives `source`, type, adapter/capabilities, canonical draft config, current revision when saved, and typed test callbacks; it **must not** own Provider CRUD or Session state. Provide distinct child controls per modality, not a single textarea for ASR, LLM and TTS.

**ASR:** Record/Pause/Stop; waveform or level meter; optional playback of captured clip; elapsed/duration; transcript + language + latency/RTF. Request `getUserMedia` only after explicit click; stop all tracks on close/unmount; Web Audio encode mono PCM16 WAV at adapter sample rate, bounded to 30s; reject browsers without mic access with an actionable message. No automatic VAD/ASR continuous session.

**LLM:** prompt textarea, Run/Cancel, response content and elapsed; tool-free single turn. Disable button on empty/oversize input.

**TTS:** text textarea, optional voice/language selectors from adapter capabilities, Run/Cancel, audio player, duration and elapsed; revoke Blob URLs.

Show 4 mutually exclusive UI test states: `idle`, `running`, `success`, `error`, plus `stale` marker whenever fields/media/credentials differ from the tested snapshot. Config hash/fingerprint in browser is **not** a credential fingerprint; only an in-memory monotonic form-change version or UI comparison, never persisted/logged.

Create flow: (1) Type & Adapter → (2) Config & optional write-only API key → (3) **Test & Review**, with Back / Test / Create buttons. A failed test should not automatically prevent saving *valid* desired configuration; show explicit warning instead. Testing never calls `create()`. Replace text implying "save and restart to test": managed runtime may prepare without process restart.

Provider Detail: keep existing modal layout unless a separate product redesign is requested; replace `runTest()` fake status logic and show real result. Show **Runtime Status** and **Last Test Result** independently. A successful draft test must not alter production runtime badges.

**Fix current edit data loss:** create/reuse `ProviderConfigEditor.vue` powered by backend `config_schema` and full `AdminProvider.config_json`; do not reconstruct typed config solely from store `model/description/endpoint`. Keep non-secret canonical fields intact; API key remains write-only and is never pre-filled from masked metadata.

### 5.2 MCP list and new detail page

Keep existing `/mcp` list (responsive 2–3 column cards, `studio-panel`, `PageHeader`, existing theme tokens and action menu). Make each MCP card a clear accessible link/click surface to **`/mcp/:key`** (RouterLink, focus ring, pointer cursor, Enter/Space behavior); place Edit/Toggle/Delete controls outside the link surface to avoid nested interactive elements. Provide a "View details" affordance/chevron and do not trigger card navigation from the action buttons.

Create `apps/admin-web/src/pages/mcp/McpServerDetailPage.vue` with this information hierarchy (avoid another giant modal):

```text
MCP Servers  /  weather-service
┌────────────────────────────────────────────────────────────┐
│ [Server icon] Weather Service               [Edit] [···]  │
│ Streamable HTTP · Enabled · Key: weather-service           │
│ Safe endpoint origin/path · Bearer · key ****••••          │
└────────────────────────────────────────────────────────────┘

┌────────────────────────────┐ ┌───────────────────────────┐
│ Connection                 │ │ Discovery                 │
│ Last manual test: --       │ │ Last discovered: --       │
│ [Test connection]          │ │ [Discover tools]          │
│ Handshake / latency / err  │ │ N tools, complete/failed  │
└────────────────────────────┘ └───────────────────────────┘

[Tools]   [Configuration / Usage]
Search tools...                                     N results
┌────────────────────────────────────────────────────────────┐
│ get_weather                 external.weather.get_weather   │
│ Look up forecast data                                     │
│ [Input schema ▾]           [Discovered; approval separate] │
└────────────────────────────────────────────────────────────┘
```

**Header:** compact, aligned with `PageHeader`/other Detail pages; show display name, immutable key, transport, desired enabled status and masked credential metadata. Render safe endpoint origin+pathname only (reuse current `safeEndpoint` or `redactEndpoint`), never query, userinfo, auth, token. Keep actions Edit, Enable/Disable, Delete in existing convention; change actions honor `If-Match` revision.

**Connection card:** button `Test connection` performs explicit one-shot saved probe; status `Not tested`, `Connecting`, `Connected (at test time)`, `Failed`, elapsed and bounded failure reason. No automatic probe on page load/refresh and no permanent `online` badge based on stale results. Provide manual retry.

**Discovery card:** `Discover tools`, loading/progress (no fake percent; show "Initializing" / "Reading tools" if backend exposes stage), successful tool count, response time, no partial result shown on a failed page. Distinguish `0 tools` from a network failure. On a new probe, clear or mark prior catalog stale until a fresh complete result; do not merge catalogs from different snapshots.

**Tool Catalog:** search by tool name/description, optional expansion for bounded pretty-printed JSON input schema, original name and `external.<server_key>.<tool>` display. Show validation/dropped counts only when meaningful. Descriptions from remote are untrusted: render **as text**, never `v-html`; JSON likewise escaped/preformatted without unsafe parsing. Explicit disclaimer "Discovery ≠ Agent approval"; optional link to Agent Tools review page, not an Enable button on discovered item.

**Configuration / Usage:** safe URL, auth type/header name, connect/request timeout, masked credential/version, desired enabled state, bound Agents (only if existing Admin binding read can produce it cheaply; otherwise show link to Agents rather than invent a new heavy query). No editable credentials in GET detail.

`McpServerFormModal.vue`: add a compact `Test & Discover` area usable **before Create** and on **unsaved Edit**; use draft probe routes and either inline key or saved credential identity. Preserve form values when test fails; changing URL/auth/key invalidates results. Show connection handshake first, then discovery (separate buttons; optionally enable Discover only after connection success, but allow discover to perform its own full handshake). Saving must remain independent from testing.

**Responsive:** desktop max content width of established Detail pages, cards side-by-side at `lg`, stacked on mobile; consistent border, muted typography, studio accents, skeleton/loading/error states, dark-mode tokens. Avoid excessive header height, giant status banners, nested card controls, destructive primary CTA. The `/mcp` page should retain overall visual language rather than creating an unrelated theme.

### 5.3 Web interfaces/files

- `apps/admin-web/src/api/providers.ts`: add `testDraftLlm`, `testDraftTts`, `testDraftAsr`; use `requestJson`, `requestBlob` and proper `FormData`. Add response header helper for TTS metrics without breaking existing `requestBlob` callers.
- `apps/admin-web/src/api/types/providers.ts`: typed discriminated test DTOs; inline write-only `api_key` as optional input only, never part of read model.
- `apps/admin-web/src/components/providers/ProviderTestPanel.vue` (**new**); optional internal `AsrMicRecorder.vue`, `TtsAudioResult.vue` only if substantive behavior warrants separate modules.
- `apps/admin-web/src/components/providers/ProviderConfigEditor.vue` (**new** or extract existing fields), preserving full canonical config.
- `apps/admin-web/src/components/providers/ProviderCreateDrawer.vue`, `src/components/admin/ProviderDetailModal.vue`, `src/components/admin/ProviderFormModal.vue`, `src/stores/admin.ts`, `src/views/ProvidersView.vue` integrate without duplicating test logic.
- `apps/admin-web/src/api/mcp.ts`: `testDraftConnection`, `discoverDraftTools`, `testSavedConnection`, `discoverSavedTools`.
- `apps/admin-web/src/api/types/mcp.ts`: `McpProbeConfig`, `McpProbeResult`, `McpToolDescription`, `McpDiscoveryResult`; keep `AdminMcpServer` and credential metadata safe.
- `apps/admin-web/src/components/mcp/McpDiagnosticPanel.vue` (**new**), `McpToolCatalog.vue` (**new**), `McpServerFormModal.vue`, `McpServersView.vue`.
- `apps/admin-web/src/pages/mcp/McpServerDetailPage.vue` (**new**) and `src/router/index.ts`.
- `apps/admin-web/src/i18n/*`: ensure all added labels/errors have both current supported locales; use existing i18n structure.

## 6. Backend file/change map

| Change | Target |
|---|---|
| New Provider source-aware test runner / draft DTO | `src/services/provider_diagnostic.rs` or `src/services/provider_test.rs` (one entry interface) |
| Draft Provider identity/acquisition/expiration | `src/services/provider_runtime/{identity,registry,lease,lifecycle}.rs` |
| Explicit source materialization; no id==0 draft trick | `src/services/provider_runtime/factory/mod.rs`, `src/providers/database_loader.rs` |
| Shared typed config/secret resolution | `src/database/provider_config.rs`, `src/database/{providers,credentials,secrets}.rs` (DB-owned read only) |
| Draft HTTP handlers and shared WAV parser | `src/app/admin/provider_tests.rs` + optional `provider_test_drafts.rs` |
| MCP connection/catalog diagnostic runner | `src/tools/external_mcp/{diagnostic,client,manager,registry}.rs` |
| Saved MCP snapshot read | `src/database/mcp_servers.rs` (no ad hoc SQL in HTTP handler) |
| MCP diagnostic HTTP routes / DTO / errors | `src/app/admin/mcp_tests.rs` (**new**), `src/app/admin/mod.rs` |
| State-owned dependencies and limits | `src/app/state.rs`, `src/config/{mod,defaults,validation}.rs` |
| Admin OpenAPI/Postman contract snapshot | `docs/api/00-all-apis.postman_collection.json` (no real key!) |
| Documentation cross-links | Relevant `docs/` index; preserve ADR 0056/0069/0071/0083 semantics |

**Deletion test for architecture:** Removing `ProviderTestRunner` should reintroduce source/credential/runtime/cleanup logic across handlers; removing `McpDiagnosticRunner` should reintroduce credential, RMCP handshake, bounded pagination and catalog policy across handlers. Avoid a shallow file which only forwards to distinct duplicated implementations.

## 7. Test plan — TDD at actual interfaces

### 7.1 Server Provider

1. Draft LLM with deterministic qualification adapter returns exact text **without** inserting provider; same adapter/typed config path as saved LLM.
2. Draft ASR accepts mono PCM16 WAV at advertised sample rate and returns text + language + accurate duration/RTF; rejects invalid MIME, unsupported sample rate/channel/format, 0 Hz, oversized/too-long clips and malformed multipart.
3. Draft TTS returns parseable WAV with valid RIFF header/sample metadata; no DB/audio persistence; voice/language mismatch rejected.
4. Full required/optional typed validation, canonicalized config, prohibited embedded credential key, adapter-kind mismatch, unknown/uncompiled adapter fail safely.
5. Draft with inline API key passes it only to the exact request-scoped remote client and never to SQLite/response/log; saved-credential+inline conflict rejected; stale saved credential revision gives 409; no cross-resource credential adoption.
6. `ResourceKey` and global work quotas stay bounded under simultaneous saved + draft requests; same compatible local native asset may share only when permitted; remote credentials remain isolated.
7. Timeout and cancellation preserve terminal ACK/quarantine; no early capacity return; memory reservation or model load cannot outlive owner unaccounted; clean shutdown.
8. Saved `/providers/{key}/test/*` output and error compatibility; disabled/not-found behavior unchanged. New draft success never upgrades production readiness.
9. No raw input (mic audio, transcript, prompt/LLM response), token, encryption ciphertext or full user destination in telemetry.

### 7.2 Server MCP

Use deterministic mock Streamable HTTP server and RMCP protocol (no real Internet in tests):

1. Draft `connection` performs `initialize` + initialized notification, emits success even when tools list empty; **zero `tools/list`** for connect-only and zero `tools/call` for all probes.
2. Draft `discover` handles stateless JSON and session-id/SSE, multiple pages/cursor, correct final catalog/LLM-visible names, no duplicate/partial results.
3. Saved connection/discover use encrypted credential (or fallback) server-side; masked metadata only; absence/wrong credential yields bounded failure; no record mutation.
4. Inline draft key remains ephemeral; switching auth to `none` forbids key; header mode checks allowed non-protected header names; edit-draft saved credential compatibility/revision enforced.
5. Input validation matches real MCP create: URL scheme/host allowlist, userinfo/query/fragment restrictions, no redirect following, TLS verification; preserve ADR 0056 HTTP-LAN policy.
6. Tool schema rejection, renamed/dropped invalid tool names, collisions, pagination cap, description/schema bytes cap, overall time limit; no partial catalog considered successful.
7. Probe semaphore/budget rejects overload; shutdown gate blocks new work; canceled clients do not retain unbounded RMCP sessions.
8. Discovery MUST NOT create binding, approve tool, advance review fingerprint, change Agent policy or mutate active Session catalog; CRUD auth change continues to invalidate approval as before.
9. Connect latency, failure codes and counts contain no credential, full URL, tool payload, raw remote body, or unbounded text.

### 7.3 Web

- Provider Create step 3 tests before save; failed test preserves draft; create CRUD called only after explicit Create click.
- Provider Detail performs **actual** API call (delete fake status-based `runTest()`); labels `Runtime` and `Last test` independently.
- Provider Edit reads/writes **complete config_json** without losing adapter-specific fields; stale results reset on edits; API key never prefilled or persisted.
- ASR mock `getUserMedia` permission denied/stop/unmount/encoding; correct WAV headers, mic track cleanup, duration cap, no concurrent recording.
- TTS playback uses Blob URL and cleanup, reliable content-type/metrics/error behavior; LLM loading/cancel/retry UI.
- MCP list card click opens `/mcp/:key`; child action buttons never navigate; keyboard accessibility and return link verified.
- MCP form draft connection/discovery work before save; saved detail buttons work; failure/retry/stale states; no auto-discovery on GET/detail mount.
- MCP catalog renders escaped text/JSON, search/expand, complete marker/tool count, zero tools empty state, and "not approved" message; no `tools/call` action.
- Test skeletons in `src/api/*.test.ts`, `src/components/providers/*.test.ts`, `src/components/mcp/*.test.ts`, `src/pages/mcp/*.test.ts` covering request shapes and 401/400/409/413/429/502/503/504.
- Light/dark responsive UI; no click propagation bugs and no raw secrets in snapshots or error messages.

## 8. Implementation sequence (small tracer bullets)

| Ticket | Description | Depends on | Gate |
|---|---|---|---|
| T1 | Define input/output/error contracts, secrets modes, request caps, fixture tests | — | No secret in logs/DTO |
| T2 | Refactor Provider source identity and materialization seam; add draft lease acquisition | T1 | Production runtime regression tests |
| T3 | Implement source-aware Provider runner + draft HTTP routes; shared ASR parser | T2 | Server integration + qualification adapters |
| T4 | Implement MCP diagnostic seam reusing RMCP; saved snapshot DB read + draft credential path | T1 | Deterministic protocol tests, zero tools/call |
| T5 | MCP Admin probe routes, rate/timeout/error mapping, API/Postman docs | T4 | HTTP integration tests |
| T6 | Shared ProviderTestPanel + codec; Create & Detail/unsaved Edit integration; config preservation | T3 | Vitest, typecheck, browser smoke |
| T7 | MCP detail page + catalog panel + clickable list cards + form draft probes | T5 | Vitest, responsive/accessibility |
| T8 | Cross-feature cancellation, resource quotas, credential security, review authorization regression | T6,T7 | End-to-end acceptance + no policy regressions |

Suggested commands from project root (adjust exact Cargo package/features to repo CI):

```bash
cargo fmt --all -- --check
cargo test -p voice-agent-server --lib
cargo test -p voice-agent-server --test admin_api
cd apps/admin-web && npm ci && npm test && npm run typecheck && npm run build
```

Also run existing targeted MCP admission/allowlist and provider-runtime integration test targets, qualification feature where CI enables it. Mock tests must be deterministic and local; real external credentials/network belong only in opt-in qualification gates.

## 9. Acceptance checklist

- [ ] Unsaved LLM draft returns actual text; unsaved ASR mic returns actual transcript; unsaved TTS returns playable audio.
- [ ] Saved Provider Detail uses real execution and correct runtime/test distinction; no fake pass/fail check remains.
- [ ] No Provider row, config mutation or credential ciphertext is written during draft tests.
- [ ] All Provider local tests honor native resource identity, memory budget, timeout and lease accounting.
- [ ] MCP Create form tests connection and discovers tools without saving.
- [ ] MCP Detail `/mcp/:key` has explicit Connect and Discover buttons, safe header, complete tool catalog and responsive card layout.
- [ ] MCP calls exclusively `initialize` / paginated `tools/list` for probing, never `tools/call`.
- [ ] MCP tests do not bind Agents, approve tools, or modify observed-review authority; Existing External MCP authorization remains valid.
- [ ] Current encrypted Admin credential semantics and environment fallback remain compatible.
- [ ] Updated Postman collection and web typed clients match exact routes/response variants.
- [ ] All regression/contract tests pass and error/log redaction is verified.

## 10. Useful source links (pinned revision)

- [Provider diagnostic handlers](https://github.com/hailp-vn38/ai-agent-voice/blob/37641bc0a63428a038904c2acd40ada949db88bf/crates/voice-agent-server/src/app/admin/provider_tests.rs)
- [Diagnostic execution/lifecycle](https://github.com/hailp-vn38/ai-agent-voice/blob/37641bc0a63428a038904c2acd40ada949db88bf/crates/voice-agent-server/src/services/provider_diagnostic.rs)
- [Provider runtime manager](https://github.com/hailp-vn38/ai-agent-voice/blob/37641bc0a63428a038904c2acd40ada949db88bf/crates/voice-agent-server/src/services/provider_runtime/registry.rs)
- [Source-aware materialization locations](https://github.com/hailp-vn38/ai-agent-voice/blob/37641bc0a63428a038904c2acd40ada949db88bf/crates/voice-agent-server/src/providers/database_loader.rs)
- [Admin MCP handlers](https://github.com/hailp-vn38/ai-agent-voice/blob/37641bc0a63428a038904c2acd40ada949db88bf/crates/voice-agent-server/src/app/admin/mcp_servers.rs)
- [MCP manager and discovery](https://github.com/hailp-vn38/ai-agent-voice/blob/37641bc0a63428a038904c2acd40ada949db88bf/crates/voice-agent-server/src/tools/external_mcp/manager.rs)
- [MCP RMCP client](https://github.com/hailp-vn38/ai-agent-voice/blob/37641bc0a63428a038904c2acd40ada949db88bf/crates/voice-agent-server/src/tools/external_mcp/client.rs)
- [MCP tool schema and names](https://github.com/hailp-vn38/ai-agent-voice/blob/37641bc0a63428a038904c2acd40ada949db88bf/crates/voice-agent-server/src/tools/external_mcp/registry.rs)
- [Admin Web MCP list](https://github.com/hailp-vn38/ai-agent-voice/blob/37641bc0a63428a038904c2acd40ada949db88bf/apps/admin-web/src/views/McpServersView.vue)
- [Admin Web Provider Create](https://github.com/hailp-vn38/ai-agent-voice/blob/37641bc0a63428a038904c2acd40ada949db88bf/apps/admin-web/src/components/providers/ProviderCreateDrawer.vue)
- [ADR 0083: Encrypted resource credentials](https://github.com/hailp-vn38/ai-agent-voice/blob/37641bc0a63428a038904c2acd40ada949db88bf/docs/adr/0083-admin-managed-resource-credentials.md)
- [ADR 0056: External MCP outbound policy](https://github.com/hailp-vn38/ai-agent-voice/blob/37641bc0a63428a038904c2acd40ada949db88bf/docs/adr/0056-external-mcp-outbound-network-policy.md)

---

**Agent instruction:** Implement test-first by ticket. After each ticket, run the targeted tests and verify invariants; do not broaden to tool invocation, raw credential read, new persistent test result schema, Device MCP policy or a redesign of unrelated pages. If a source contract differs in a newer commit, re-inspect `main`, record the divergence, and update this guide before changing code.
