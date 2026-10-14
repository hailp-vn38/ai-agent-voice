# v1.4.2 -> v1.5 change log and external reviewer closure

| Topic | v1.4.2 | v1.5 |
|---|---|---|
| Plugin implementation | `mcp_binding`, manifest maps to reviewed MCP Tool | **Executable Python 3.12 source, uv.lock dependencies, isolated container worker**, no MCP at dispatch |
| Package install | Curated manifest referenced from repository; no installed package filesystem | Admin ZIP / curated ZIP, validated extraction, isolated dependency image build, immutable package + OCI digest in filesystem and SQLite |
| Permission | Reserved background Agent + existing MCP approval | Admin-reviewed per-package operation/egress policy, host-enforced, no per-Agent grants or MCP review reuse |
| Operation execution | Guarded external `tools/call` | Bounded JSONL request/response, one container/Step attempt, artifact import from constrained workspace |
| Size bounds | Existing MCP result cap 16–64 KiB | Own worker stdout cap + filesystem artifact quota; no borrowed MCP result limits |
| Dependency failures | MCP transport/tool invocation -> `error` | Plugin-specific import/runtime/image/protocol -> `error`; third-party HTTP errors are Step failures, not global plugin poisoning |
| Retry | Manual new Job; no auto tool retries | Retained; opaque executable code never gives exactly-once guarantee |
| Plugin options | Step operation pin after planner | **Frozen Plugin Choices committed on Job submit**, including image/permission digests (schemas covered by package digest); replan cannot expand |
| Recovery `cancelling` | Not specified | **Startup terminalizes to `cancelled`**, retains unknown worker effects and cleans labeled orphan workers |
| Device grant at dispatch | Only Agent explicitly checked | **Live Device incarnation/enabled/original assignment plus Agent enabled (no background toggle)**; mutation + actual worker start synchronized under publication lock |
| Terminology | Duplicate `Job Owner Scope` and `Job Requester Scope` | Only **Job Requester Scope** canonical |
| Job engine/provider/Agent/Web/Reminder | Durable DAG, global 2 LLMs, Agent boolean, Manual Retry, no notifications | Durable DAG/2 LLMs/Manual Retry retained; **Agent boolean removed**; no changes to Reminder or Direct TTS |

**Out of V1 intentionally:** arbitrary/untrusted public plugin submissions, background plugin credential injection, npm/JS/native Rust/WASM plugin runtimes, warm worker pool, dependency install at invocation, remote HTTP plugin store, distributed scheduler, auto retries, push delivery, recurring task scheduler.

**Safety caveats:** An Admin-trusted plugin with container isolation is not proof against hostile code; validate network egress, build isolation and host runtime policy in deployment. A plugin classified as `safe_read` may still have remote side effects; user/operator must not interpret it as exactly-once.

## Updated Voice admission decision (2026-10-13)

- **Removed** per-Agent `background_enabled` column, permission checks, Agent CRUD change and Agent UI toggle. All admitted Agents share global Background capability.
- **Added** one LLM-visible server builtin `server_create_job` registered at successful Device WS admission. The LLM calls it during a Tool Round; the connection itself never submits a Job.
- **Added** bounded async submission completion, durable success/failure ToolResult examples, server-derived requester/idempotency, concurrency and failure tests. Voice V1 does not add get/cancel tools, MCP bridge, new WebSocket job frames or notification channels.
- **Revised** ADR 0092/0097, README, Glossary and acceptance matrix accordingly.

## v1.5.1 — Ponytail and executable E2E closure

- Removed duplicate Voice-only diagram and parallel phase table; `IMPLEMENTATION-PHASES.md` now owns phase gates.
- Explicitly wired `AppState -> SessionRuntimes -> SessionActor`, ToolCall -> bounded async Job submit -> typed completion -> original Tool Round.
- Fixed contract: `server_create_job` always selects `LlmThenTts` even for Silent/DirectTts Device MCP policy; maintain JSON error semantics and reserved-name precedence.
- Added scheduler periodic fallback, terminalization for unschedulable/deadline queue, isolated installer semaphore/deadline, and separated network=none vs restricted proxy network.
- Added durable `start_requested` before container start to preserve ambiguous crash outcomes, and real golden-path/Voice-load/egress qualification.
- Added `END-TO-END-WIRING-AUDIT.md`. No code or runtime tests were performed; P3 is earliest functional E2E, P4 is qualification.
