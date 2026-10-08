# MCP Tool Authorization Refactor — Implementation Guide (v2)

> Repository: `hailp-vn38/ai-agent-voice` · Baseline: `main`, reviewed 2026-10-08.
> Status: implementation instructions; implementation began 2026-10-08.
> Supersedes the longer v1 guide following Ponytail Review: same behavior, fewer branches and less duplicate guidance.

## 1. Decisions and invariants

**Device MCP:** Every structurally valid, unambiguous tool advertised by the **currently admitted Device** is available without Admin approval. This is independent of Speaker Policy (`off`, `observe`, `required`). Remove Device tool review, recovery, per-tool allowlist and dangerous-name filtering. A Device must still pass WebSocket admission and complete MCP discovery; tool calls must stay within its own session and normal turn limits.

**External MCP:** Only tools explicitly approved for the **Agent**, with the **current observed contract fingerprint**, are visible/callable. This is mandatory in all Speaker modes, including `off`. Discovery is not approval. A production External tool call **requires `&ExternalToolGuard`**, never `Option` or an unguarded fallback. Keep sensitive tools blocked without the separately designed confirmation flow.

**Speaker Recognition:** Agent Speaker Policy still gates voice turns (particularly `required`) and Speaker/Template authority remains unchanged. Speaker mode must not decide MCP approval. Template switching must not widen the pinned External tool catalog.

| Speaker Policy | Device MCP | External MCP |
| --- | --- | --- |
| `off` | All valid discovered tools | Current Agent approval required |
| `observe` | All valid discovered tools | Current Agent approval required |
| `required` | All valid discovered tools **after normal voice-turn gate** | Current Agent approval required **after normal voice-turn gate** |

Target flow:

```text
Admitted Device WS ── MCP tools/list ── validated session catalog ── LLM ── same WS tools/call
Agent External MCP ── discovery/observation ── Admin approval ── pinned guarded catalog ── LLM ── guarded remote call
Speaker gate ── authorizes voice turn, not tool approval
```

**Intentional behavior change:** Device tool names such as `self.reboot`, `self.factory_reset`, `shell.*` or `command.*` are no longer filtered by name. The operator trusts the firmware of an admitted Device. Keep schema/name collision validation, timeout, request/response correlation, turn budget and session isolation; these are protocol protections, not a permission layer. `[mcp].enabled=false` and client `features.mcp=false` still disable Device MCP discovery.

## 2. Existing defects to close

1. **E1 — External bypass:** `Database::review_external_catalog` reads `agent_speaker_policies.mode`; for `off`, `!participating || allowed` publishes unapproved External tools. **Fix:** remove Speaker lookup/`participating`; publish only currently approved contracts.
2. **E2 — Unguarded External dispatch:** `ExternalMcpClient::call_tool_guarded` accepts an optional guard and can initiate a remote request without authorization. **Fix:** require `&ExternalToolGuard` at the production dispatch seam; no bypass through `call_tool()`.
3. **D1 — Lost Device review profile:** `EffectiveSessionProfile::into_admitted_profile()` does not forward `device_tools` to the WebSocket/actor. **Resolution under new policy:** delete the unused Device review profile/guard wiring entirely, **not** a dummy forwarding patch.
4. **D2 — Device guard/invalidations not enforced:** Device dispatch does not check `DeviceToolGuard`, and WebSocket does not monitor its cancellation. **Resolution:** delete obsolete Device authorization and cancellation paths; retain existing WS/session/turn safety checks.
5. **D3 — Device recovery/observation inconsistencies:** disappeared tools and reconnect recovery are difficult to reconcile. **Resolution:** delete Device review/recovery persistence and jobs; preserve normal live discovery behavior.
6. **D4 — Legacy Device filter:** `visible_tools(discovered, mcp.allowed_tools)` and `is_dangerous_tool` contradict all-tools availability. **Fix:** expose every valid unique discovered tool without those filters.

## 3. Stage A — P0: External approval always enforced

**Files:** `crates/voice-agent-server/src/database/tool_allowlist.rs`, `src/database/tool_security.rs`, `src/tools/external_mcp/client.rs`, `src/session/actor/tools/external.rs`, `src/app/state.rs`, `src/session/profile.rs`, `src/app/websocket.rs` (paths relative to `crates/voice-agent-server` unless already rooted).

1. In `review_external_catalog`, remove the `agent_speaker_policies` query and the `participating` branch. Continue complete observation, contract fingerprinting, reconciliation of removed/changed tools, CAS/revision and audit. The publication condition is **`allowed` only**, with `sensitive=0` and current exact contract match.
2. Resolve Agent External bindings at admission and pin **only** approved `(server_key, original_name, server_id, fingerprint)` entries. On DB/guard failure, return an **empty External catalog**; voice admission may remain fail-soft, but it must never expose unapproved tools.
3. Make `ExternalMcpClient::call_tool_guarded` take **`&ExternalToolGuard`** (not `Option`). Migrate production callers. Restrict/remove any unguarded `call_tool()` production path so it cannot be used by the LLM tool executor.
4. Before **each outbound start**, use the existing `ToolSecurity` publication lock and live SQL verification of Agent binding, server state, fingerprint, `allowed=1`, `sensitive=0`. Hold the lock through request initiation, not the whole response. Keep cancellation, concurrency, limits, secret handling, network restrictions and no-retry behavior.
5. Verify `resolve_external_mcp -> SessionExternalMcp -> EffectiveSessionProfile -> AdmittedSessionProfile -> SessionActor` carries the pinned catalog **and mandatory guard**. Preserve revocation/contract-drift invalidation and WS close `1008`; new approvals apply to **new sessions**, not an existing snapshot.
6. Retain authenticated `GET/PUT /api/admin/agents/{key}/tool-allowlist`, including `If-Match`/revision conflict behavior. Do **not** auto-approve tools belonging to Agents with Speaker mode `off` during migration.

**Stage A gate:** An `off` Agent cannot advertise or dispatch an unapproved External tool; a missing guard or DB error cannot cause a network call. Previously approved current contracts still work on a new session.

## 4. Stage B — P0: Device tools come directly from discovery

**Files:** `src/tools/device_mcp.rs`, `src/session/actor/mcp.rs`, `src/session/actor/mod.rs`, `src/session/actor/construct.rs`, `src/session/actor/ingress.rs`, `src/session/actor/lifecycle.rs`, `src/session/profile.rs`, `src/app/state.rs`, `src/app/websocket.rs`, `src/config/mod.rs`, `src/config/validation.rs`.

1. Change `device_mcp::visible_tools(discovered, allowed)` to **`visible_tools(discovered)`**. Delete `reviewed_tools`, Device-only fingerprint code, `is_dangerous_tool` authorization filter and unused imports. Keep deterministic name mapping and reject duplicate original names or sanitized-name collisions; never silently choose a different tool.
2. On **complete valid** paginated `tools/list`, set `SessionActor.mcp.visible` and `ready=true` directly. On invalid/partial/timed-out discovery, clear visible tools and leave `ready=false`. Do not spawn Device observation/recovery/approval jobs. Remove `apply_device_tools_discovery` and its completion mailbox.
3. At Device dispatch, resolve the tool against the **current actor's own visible catalog**; require MCP ready, active turn/generation and the existing execution budget. Forward only the resolved original name to the **same Device WebSocket**. Unknown, stale or ambiguous names must never be forwarded. Preserve request ID correlation, cancellation, timeouts and result caps.
4. Delete `SessionDeviceTools`, `DeviceToolGuard`, `SessionActor.device_tools`, `device_tools_tx/rx`, `DeviceToolsCompletion`, `DEVICE_TOOLS_CAPACITY`, `.with_device_tools()`, `.session_device_tools()`, `drain_device_tools_completions()` and Device-review-only `Drop` hooks. Remove `resolve_device_tools()` from `AppState`. **Do not** patch the previously lost `device_tools` profile field: delete that entire unused seam.
5. Simplify `DeviceMcpState` by deleting only the obsolete approval/filter field; retain `enabled`, `discovered`, `visible`, `pending`, `result_delivery`, `tool_delivery` and protocol state. Keep `with_device_mcp` gated by `features.mcp` and `[mcp].enabled`.
6. Remove `[mcp].allowed_tools` from `McpConfig`, config validation and examples. Remove dangerous-name rejection from `tool_policy` validation because this is a **result-delivery setting**, not tool authorization. Keep its existing duplicate/empty checks, enums and timeouts; preserve `[mcp].external` and tool-round limits. **Breaking config change:** operators must delete `allowed_tools` from deployed TOML before restart when unknown fields are rejected.

**Stage B gate:** In each Speaker mode, an admitted MCP-capable Device exposes every valid unique advertised tool (including formerly dangerous names) without DB approval. It cannot dispatch an unknown tool or a tool belonging to another session; Required Speaker still gates the voice turn.

## 5. Stage C — P1: Remove Device review persistence and API

After Stage B compiles without Device review references:

- Delete `src/database/device_tool_allowlist.rs`, `src/database/device_tool_recovery.rs`, `src/app/admin/device_tools.rs`; remove their module exports, imports, jobs and Device-only registration/invalidation from `database/tool_security.rs`. **Keep** `ExternalToolGuard`, its publication lock and External/Speaker invalidation mechanisms.
- Retire only these routes (return `404`):

```text
GET  /api/admin/agents/{key}/device-tool-allowlist
PUT  /api/admin/agents/{key}/device-tool-allowlist
POST /api/admin/agents/{key}/device-tool-recovery
```

- Keep Devices CRUD, pairing, Speaker, Template and External tool review APIs unchanged.
- Add a **new forward-only SQLx migration** using the next available migration number. **Never edit** historical `0010_device_tool_allowlist.sql` or `0011_device_tool_recovery.sql`. Drop Device-review tables child-first after checking foreign keys/triggers:

```sql
DROP TABLE IF EXISTS device_tool_recovery_members;
DROP TABLE IF EXISTS device_tool_recovery_batches;
DROP TABLE IF EXISTS device_tool_observation_history;
DROP TABLE IF EXISTS agent_device_tool_allowlist;
DROP TABLE IF EXISTS device_tool_observations;
```

- Verify both fresh DB initialization and upgrade of a populated DB; run `PRAGMA foreign_key_check`, preserve all Agent/Device/Speaker/External approvals and migration history. Back up production SQLite before rollout; export old Device review evidence first if retention is required.

**Stage C gate:** No Device approval/recovery production code, API or active DB tables remain; existing Devices and External approval behavior still works.

## 6. Stage D — P1: Web, API docs and ADR

- In `apps/admin-web/src/pages/agents/AgentDetailPage.vue`, remove `AgentDeviceToolAllowlist` and its review/recovery actions. Delete `apps/admin-web/src/components/agents/AgentDeviceToolAllowlist.vue` and now-unused API methods/types/tests/i18n. Keep `AgentToolAllowlist` (External MCP) visible for **all** Speaker modes; keep `AgentSpeakerPolicy` independent.
- Update `docs/api/tool-allowlist.md` and `docs/api/00-all-apis.postman_collection.json`: External approval is always mandatory; remove only the three retired Device review requests, not Devices CRUD.
- Update `docs/04-configuration.md`, `docs/PHASE6_DEVICE_MCP_IMPLEMENTATION_GUIDE.md` and relevant Web/API docs to remove `[mcp].allowed_tools` and old Speaker-dependent tool review behavior.
- Mark `docs/adr/0078-agent-tool-allowlist.md` **superseded** by this policy decision; retain its history. No speculative new ADR, Device inventory endpoint or optional UI workflow in this refactor.

**Stage D gate:** Agent Detail contains External review but no Device approval/recovery controls; Web build, docs and Postman match server routes.

## 7. Regression tests — add only missing cases, reuse existing suites

Retain the existing MCP protocol, session, budgets, cancellation, authentication and migration tests. Replace obsolete Device review/recovery assertions instead of building a second test harness.

| New/changed test | Required assertion |
| --- | --- |
| Device × Speaker `off/observe/required` | All valid unique tools visible; `required` still blocks unauthorized voice turns |
| Device former denylist + isolation | `self.reboot`/`shell.*` visible; only the owning WS receives `tools/call`; unknown/colliding names never dispatch |
| Device discovery failure | Partial, malformed or timed-out `tools/list` leaves no callable stale catalog |
| External × Speaker `off/observe/required` | Unreviewed tool is Admin-observable but never advertised/called; exact approval works only in new sessions |
| External guard and revocation | Missing guard/DB failure => no outbound call; sensitive blocked; changed/revoked contract blocks new starts, invalidates WS (`1008`); concurrent revoke honors publication lock |
| Agent/Template boundary | Approval does not leak across Agents; Template switch cannot widen existing External snapshot |
| Migration/API/Web | Fresh + populated DB migrate; three retired routes are `404`; External GET/PUT and Devices CRUD work; Agent Detail and Postman contain no retired controls/routes |

Suggested verification (adjust only to the repository's actual scripts):

```bash
cargo fmt --all -- --check
cargo test -p voice-agent-server
cargo clippy -p voice-agent-server --all-targets -- -D warnings
cd apps/admin-web
npm run typecheck
npm run test
npm run build
```

Run focused MCP tests first; record actual pass/fail output in the implementation PR. This document does not claim that these tests have been run.

## 8. Definition of done

- [ ] **Device:** Every valid discovered tool is available without approval, independent of Speaker; calls stay within the admitted Device session and normal turn controls.
- [ ] **External:** Every outbound call requires a current per-Agent approval **and mandatory live guard**, independent of Speaker; revoke/drift blocks new starts.
- [ ] **Flow:** The broken Device review profile/actor path and all dead review/recovery paths are deleted; External pinned profile/guard survives admission, WS and dispatch.
- [ ] **Delivery:** Forward migration, server/API, Vue, config, Postman, docs and regression tests are synchronized; test results and changed files are reported.

**Implementation rule:** Prefer deleting obsolete Device review code over adding compatibility adapters or a generic permission engine. Deliver Stages A and B together so the External bypass is not left deployed. Report changed files, tests run, failures and deferred items.
