# End-to-end wiring and bottleneck audit — v1.5.1

**Scope:** Original static audit, updated after P0 implementation. **CURRENT STATE = P0 FOUNDATION / FULL E2E NOT VERIFIED**. Migration, shared runtime, deployment lock, durable admission/recovery, periodic scan, Admin Background/Jobs handlers and Web pages exist. Voice builtin, LLM planner, Python runtime and sandbox/load gates remain pending. [P0 evidence](P0-IMPLEMENTATION.md) distinguishes implemented foundation tests from later execution gates.

## Main pipeline and producer-consumer status

| Seam | Existing source | New producer -> consumer | Current evidence / release blocker |
|---|---|---|---|
| Boot and ownership | `app/mod.rs`, `app/state.rs`, `RuntimeLifecycle` | Open migrated SQLite -> initialize shared BackgroundRuntime -> one Job scheduler -> inject submit handle to WebSocket actors | **P0 implemented:** shared runtime, periodic scan, startup recovery and file lock. Session submit injection belongs to P3. |
| Plugin packaging | No executable plugin installer | Admin ZIP -> validator -> bounded isolated uv builder -> package/image store -> SQLite `disabled` -> Admin Activate -> `ready` | **Missing** installer, rootless runner/preflight and policy enforcement. Networked plugin must be blocked without egress. |
| Tool visibility | `tools/builtin.rs`, `session/actor/tools/catalog.rs` | Successful physical or Studio WS admission -> immutable `server_create_job` definition -> tool-capable Voice LLM | **Missing** builtin wiring. Connection alone must create 0 Jobs. `features.mcp=false` not relevant. |
| Tool execution | `session/actor/tools/{round,builtin_actions}.rs`, `session/actor/ingress.rs`, `session/event.rs` | Real LLM call -> bounded async submit -> matched completion -> `complete_tool_call` -> continuation LLM/TTS | **Missing** async event and Actor handling. Existing default MCP result delivery cannot determine builtin behavior; must explicitly force LlmThenTts. |
| Durable handoff | SQLite `database/mod.rs` | JobEngine submit -> idempotent transaction/queue -> scheduler scan/wake -> pinned two LLM providers -> DAG steps | **P0 schema/admission implemented; executor pending.** Production submit rejects `runtime_unavailable`; only the foundation harness can queue. |
| Python execution | No Python container plugin runtime | Job step -> live authorization+durable attempt -> rootless container -> JSONL -> checked Job artifacts -> terminal CAS | **Missing** worker, builder, artifact store and reaper. Image/permissions pinned; no MCP bridge. |
| Result observation | Admin framework and Vue Router exist | DB terminal Job -> Admin `/jobs` -> `/jobs/{id}` -> authenticated artifacts | **P0 Background/Jobs handlers and Web pages exist.** Artifact download/terminal execution and Voice creation remain P2/P3. |
| Revocation/Crash | `database/tool_security.rs::publication` exists | Device/Agent/global/plugin mutation -> write lock; worker authorization+start -> read lock; process restart -> orphan recovery | **P0 mutation publication and DB recovery implemented.** Actual container start/reaper qualification remains P2/P4. |

## Priority blockers prior to claiming feature complete

Priority labels below are the original audit severity, not implementation-phase numbers. P0 scheduler liveness and database admission now pass foundation tests; automatic planning, actual worker starts and Voice completion fences remain later-phase gates.

1. **P0 wiring gap:** `AppState -> SessionRuntimes -> SessionActor` submit handle and typed completion across actor ingress are not already implemented. The Voice builtin cannot work just because a name is added to ToolDefinition.
2. **P0 end-to-end phase gap:** P2 can run code via a test seam but cannot produce a fully planned Job until P3 adds orchestrator and worker LLM. Only P3 passes first complete tracer; P4 qualifies deployment reliability.
3. **P0 tool result mismatch:** `round_result_delivery` defaults to Device MCP delivery for non-MCP builtin names. Special-case `server_create_job` as `LlmThenTts`, preserve `isError`, avoid MCP direct-response parsing and check stale session/turn/generation/call IDs.
4. **P0 scheduler liveness:** Atomic DB queue admission alone cannot ensure processing. Must have process-owned scheduler, durable scan fallback after lost notify, deadline terminalization and no infinite waiting on revoked/unavailable provider/plugin.
5. **P0 network deploy blocker:** For `weibo.com` crawler, a rootless container with `--network=none` cannot reach a proxy. Enforce a restricted isolated network with only allowlisted proxy egress, and validate DNS/rebinding/private routes. Until deployed, keep networked plugin disabled.
6. **P1 build/install head-of-line blocking:** `uv` image build can be expensive and execute dependency hooks; use dedicated one-slot bounded installer semaphore, finite timeout, isolated builder and no long DB locks or Voice permits.
7. **P1 shared resource pressure:** Separate provider admissions for non-Voice and maintain two bounded container workers; use WAL/short transactions and measurable queue wait/DB commit/container launch latency to establish no Voice starvation under load.
8. **P1 crash/start ambiguity:** DB reservation cannot atomically commit with Docker start. Persist start_requested before start; on crash/timeout conservatively mark unknown_outcome, kill owned orphans and never auto-retry.
9. **P1 Voice capability:** Every Agent is eligible, but an LLM with no function/tool-call support cannot invoke a tool. This must be detected/reported; no fallback that invents background execution from WS connect.
10. **P2 UI limitation:** Without a new WS event the floating client cannot reliably extract the `job_id` except via LLM text. This is acceptable for V1; Admin `/jobs` remains the authoritative listing; do not assert guaranteed clickable receipt in the Floating Chat.

## Final end-to-end qualification scenarios

- **Network-free golden path:** Install fixture-echo ZIP -> approve+activate -> set global provider pair -> connect managed physical/Studio WS -> LLM calls builtin -> tool returns durable queued receipt -> LLM speech confirms queue -> scheduler plans -> real Python 3.12 worker produces one artifact -> Worker LLM summarizes -> Job `succeeded` -> Admin detail downloads artifact. Force Device `features.mcp=false` and Device MCP output DirectTts/Silent to ensure builtin reply survives.
- **No orphan path:** WS disconnect after commit, kill server just after `start_requested`, restart under same DB and Docker label namespace; Job terminalizes as interrupted/unknown_outcome, owned containers reaped, no automatic replay and no stuck `cancelling`.
- **Revocation path:** Race Device disable/delete/reassign and Agent disable/delete with actual worker start. After mutation commit no new starts; in-flight may require kill, unknown effect evidence persists. Verify all actual mutation implementations use publication coordination.
- **Throughput path:** Run simultaneous Voice ASR+TTS, Reminder due and two Background workers, then queue extra Jobs and one installer build; capture p95 WS responsiveness, scheduler queue wait, SQLite transaction latency, CPU/RAM caps, provider wait and cancelled job terminal time. Declare test budgets against baseline before run; document FAIL/NOT_RUN rather than inventing performance numbers.
- **Networked path:** Test crawler behind enforced egress: allowed site works, unapproved DNS/redirect/private IP attempts fail, `network=none` fixture remains isolated. No claim of web crawling readiness from fake proxy/network-only mocks.

## Release decision

**No current proof of operational end-to-end success.** Functional readiness requires P3 golden path PASS. Production/deployment readiness requires P4 load/crash/egress/recovery PASS for the enabled plugin profiles. Static spec self-consistency checks can pass now, but no cargo/npm/container tests should be reported as having run.
