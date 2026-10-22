# Core Plugin Policy V3 — Implementation and Acceptance

Status: Implemented in branch `feat/agent-job-v3-core-plugin-policy`, based on `feat/agent-job-v3-opencode-session-runtime`. Related: [ADR 0115](adr/0115-core-plugin-activation-policy.md), [Flow 11](flows/11-core-plugin-activation-v3.md), [OpenCode V3 Flow](flows/10-agent-job-opencode-v3.md).

## Scope

Enable and disable the four statically bundled Rust Core Plugin groups via authenticated Admin API. Persist the decision in SQLite. At Job submission, include only enabled groups; at a reserved Core Step start, atomically recheck the group's live enabled flag. No fake External Plugin installations, image or new executor framework.

| Group ID (management only) | Frozen operation identifiers |
|---|---|
| `core.web` | `core.web.fetch`, `core.web.extract` |
| `core.document` | `core.document.extract`, `core.document.inspect` |
| `core.data` | `core.data.json_query`, `core.data.transform` |
| `core.artifact` | `core.artifact.read_text`, `core.artifact.inspect` |

Frozen JSON continues using `plugin_id: "core"`, `operation: "web.fetch"`, `kind: "core"`, `contract_revision`. `core.web` is **not** a frozen plugin_id. Policy revision and contract_revision are separate identities.

## Implementation map

| Seam | Implementation |
|---|---|
| Forward-only SQLite migration | `crates/voice-agent-server/migrations/0119_core_plugin_settings.sql` |
| Settings CAS and durable read | `src/database/core_plugins.rs` |
| Static group ownership, validation, tagged catalog | `src/tools/core/catalog.rs` |
| Job admission transaction and Core enabled snapshot | `src/database/jobs/submission.rs` |
| Aggregate enabled Core + ready External choices | `src/services/background/plugins/mod.rs` |
| Deterministic Step-level denial `core_plugin_disabled` | `src/services/background/model.rs` |
| Start authorization transaction | `src/database/jobs/execution.rs::request_plugin_start` |
| HTTP read/mutation | `src/app/admin/plugins.rs` and `src/app/admin/mod.rs` |
| Web types and API | `apps/admin-web/src/api/types/agent-jobs.ts`, `src/api/agent-plugins.ts` |
| Web Core section | `apps/admin-web/src/pages/plugins/PluginStorePage.vue` |
| Domain and design | `GLOSSARY.md`, ADR 0115, Flow 11 |

All paths under `src/` above are relative to `crates/voice-agent-server` unless otherwise qualified.

## Persistence and lifecycle

`core_plugin_settings` stores `plugin_id TEXT PRIMARY KEY`, `enabled INTEGER NOT NULL CHECK (enabled IN (0,1))`, `revision INTEGER NOT NULL CHECK(revision>0)` and `updated_at INTEGER NOT NULL`. Migration seeds all four groups as enabled at revision 1.

`Database::core_plugin_settings` reads current rows. `Database::set_core_plugin_enabled` uses publication write lock + `BEGIN IMMEDIATE` and a revision-guarded SQL update. Every successful write increments revision, even for the same enabled state. Unknown Core IDs fail `not_found`; missing expected rows fail closed. Restart does not overwrite stored enablement.

`tools/core/catalog.rs` maps known statically compiled operations to exactly one group; validates all four settings rows and renders catalog status. Tags are presentation-only. No new dynamic operation registry or cache invalidation bus.

## Admission and runtime authority

During `submit_job`, read Core settings in the same `BEGIN IMMEDIATE` transaction that freezes External installation pins and persists the Job. Freeze enabled Core operations only; validate required capabilities and sources **after filtering**. Idempotent Job replay uses its existing frozen JSON unchanged.

During `request_plugin_start`, after checking chosen operation and reserved Attempt, map the Core operation to its static group and read live `enabled` in the same write transaction. Disabled is `JobError::CorePluginDisabled` (`core_plugin_disabled`); the existing durable execution path records a known failed Step and matching Tool Result in Child Session. An already-committed `start_requested` Step may complete after a concurrent disable.

Do not recheck current enabled state in scheduler `job_eligible`, Job finalization, Core operation contract compilation, `mark_plugin_started`, or result publication. Disabling an unused frozen operation must not cancel an otherwise healthy Job. An old Job never gains newly enabled operations; a resumed Child retains its provider-neutral canonical messages and frozen allowed operations.

## Admin contract

```http
GET /api/admin/plugins/builtin
Authorization: Bearer <admin-token>
```

Returns `{items,total:4,operation_total:8}`. Each Core item has `id`, `name`, `kind:"built_in"`, `tags:["core"]`, `enabled`, `status:ready|disabled`, `revision`, `runtime:{kind:"in_process",language:"rust"}`, and static operations.

```http
PATCH /api/admin/plugins/builtin/core.web
Authorization: Bearer <admin-token>
If-Match: "1"
Content-Type: application/json

{"enabled":false}
```

Success returns updated Core group at revision 2, even if this PATCH repeats the current state with a valid revision. `400 invalid_if_match` or malformed JSON, `404 not_found`, `409 revision_conflict`, `503 database_unavailable`. Missing auth is `401`. The old External `/plugins` endpoint and lifecycle actions remain unchanged.

Web adds one Core section to the existing Plugin Store page, with Core label and Enable/Disable actions. It neither installs/uninstalls built-ins nor routes them to External detail. The user is informed that already-started Steps are allowed to complete.

## Acceptance tests

| Case | Expected behavior |
|---|---|
| P01 | Fresh migration seeds four enabled groups, eight operations, no Store rows |
| P02 | CAS policy disable persists; same-value PATCH increments revision; competing stale revision returns conflict |
| P03 | New Job excludes disabled operations; Web-only required capability/source fails before insertion |
| P04 | Idempotent replay and resumed Child cannot acquire newly enabled tools |
| P05 | Disable before `request_plugin_start` rejects with known failed Step, not `unknown_outcome` |
| P06 | Accepted Step can complete after a later disable |
| P07 | Unrelated Data Job remains eligible when Web group disabled |
| P08 | API authentication, missing revision, invalid body, unknown group and DB failure fail closed |
| P09 | Catalog and Web display correct Core tags/status/revisions and separate External totals |
| P10 | V3 Root → Child native ToolCall receives one matched ToolResult after disabled Step |
| P11 | Existing DAG V1 / hierarchical V2 / V3 and External Store lifecycle unaffected |
| P12 | SQLite concurrent PATCH and Step start produce deterministic commit-order semantics |

Tests reuse existing harnesses, not a new framework. Use repository wrapper (`AGENTS.md`) for Rust checks, SQLx ownership, formatting, Web typecheck and full suite.

## Code Review follow-up (2026-10-15)

- **F1:** `PATCH /plugins/builtin/{id}` now renders the committed setting directly via `core::bundled_item`. No post-commit catalog query can turn a successful mutation into a spurious 503. A regression test deletes an unrelated catalog row and proves the PATCH still reports the committed revision.
- **F2:** A concurrent test races `set_core_plugin_enabled` against `request_plugin_start` and asserts that either commit order produces the matching durable Attempt state. The existing sequential tests still prove both explicit orders.
- **F3:** Idempotent replay after re-enable preserves the original frozen operations; a Child Session resumed after enable retains its Session ID, system prompt and Job-level operation ceiling.
- **S1:** The Core catalog owns its policy view and validation without importing the database module. Persistence remains in `database/core_plugins.rs`.
- **S2/S3:** Core labels use the existing English/Vietnamese i18n catalogs. The UI does not replace a newer CAS revision with an older in-flight GET response.
- **F4:** Additional Admin API tests cover missing/invalid `If-Match`, malformed body, absent known group and fail-closed catalog reads.

## Deployment and caveats

Existing databases migrate forward; `core_plugin_settings` is not reseeded on each boot. Old frozen Jobs remain valid for their original operation set, but cannot start disabled operations. No changes to External runner/install packages, LLM adapters, source provenance or artifact ownership.

**Risk 2 remains deferred**: no new Core Web network target/SSRF/DNS/private-IP/redirect policy is added. Keep Core Web restricted to trusted deployments until it is reviewed separately.
