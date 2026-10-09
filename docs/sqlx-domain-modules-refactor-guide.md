# SQLx Domain Modules Refactor Guide

> Repository: `hailp-vn38/ai-agent-voice`  
> Baseline: `main`, inspected 2026-10-08  
> Primary crate: `crates/voice-agent-server`  
> Status: Implementation guide / proposed design; **no production changes have been made**.

## 0. Agent execution brief

Refactor the Rust server so production SQLx queries live under `crates/voice-agent-server/src/database/`, organized by **domain ownership**, while `app/admin`, `services`, and `session` invoke small, typed database interfaces. Preserve API behavior, persistence format, security, transaction boundaries, live-session invalidation, provider lifecycle, and voice processing semantics. This is a code-organization refactor, **not a schema redesign**.

**Do not:** change HTTP routes, JSON response shapes, database migrations, SQLite driver, authorization semantics, or the live audio pipeline; build a generic `Repository<T>`/DAO framework; add speculative traits or independent pool instances; move every file only for naming symmetry; hide database errors in `Option`; replace integration tests with mocks.

**Definition of done:** every production SQL statement and its transaction policy has a clear owner under `database/`; non-database production modules have no `sqlx::query*`, `sqlx::raw_sql`, or `sqlx::QueryBuilder` call sites; relevant regression tests and static SQL inventory pass; tests/fixtures may continue to use direct SQLx.

### Source-of-truth priority

1. Behavior and constraints of current `main` at implementation time.
2. Accepted ADRs and flow documents (`docs/adr/`, `docs/flows/`). In particular, ADR-0073 requires SQLite-backed device admission and database readiness.
3. Current integration tests and HTTP error contracts.
4. This guide. If a conflict is found, document it and preserve the accepted/implemented contract; do not silently change policy to fit the proposed shape.

## 1. Verified starting point

The codebase already has `src/database/mod.rs` and specialized implementations such as:

- `database/admission.rs`: snapshots the Device -> Agent -> Template -> Provider graph before WebSocket upgrade.
- `database/device_enrollments.rs`: short-lived enrollment state.
- `database/external_mcp.rs`: desired MCP binding snapshot.
- `database/tool_allowlist.rs`: observation and reviewed External MCP catalog.
- `database/tool_security.rs`: live-session dependency invalidation and an External MCP dispatch authorization check.
- `database/load_plan.rs`: provider required/optional/unbound load planning.
- `database/history.rs`: transcript capture queue, writer, retention, and storage.
- `database/provider_config.rs`, `database/external_mcp_policy.rs`, `database/secrets.rs`: validation and secret-reference support, not necessarily SQL modules.

Direct SQL remains in HTTP handlers and runtime orchestration:

| Current source | Destination / owner | Migration scope |
| --- | --- | --- |
| `app/admin/agents.rs` | `database/agents.rs` | Agent create/get/list/update; revision-sensitive mutation; audit |
| `app/admin/templates.rs` | `database/templates/mod.rs` | Template CRUD; default assignment; provider bind |
| `app/admin/templates/relationships.rs` | `database/templates/relationships.rs` | Agent/Template relationship reads and mutation |
| `app/admin/devices.rs` | `database/devices.rs` | Device CRUD; Agent/Template validation; enrollment cancellation |
| `app/admin/enrollments.rs` | `database/device_enrollments.rs` | Claim transaction and registration updates |
| `app/admin/providers.rs` | `database/providers.rs` | Provider CRUD, filtering, relationship counts, versioned writes |
| `app/admin/provider_adapters.rs` | `database/providers.rs` | Provider instance/type/revision lookup |
| `services/provider_prewarm.rs` | `database/providers.rs` | Desired provider snapshot query; leave runtime queue and coalescing in `services` |
| `services/provider_diagnostic.rs` | `database/providers.rs` | Bounded diagnostics provider snapshot lookup |
| `app/mod.rs` | `database/providers.rs` or `database/load_plan.rs` | Default Template IDs/prewarm-related DB query; no SQL in startup orchestration |
| `app/admin/mcp_servers.rs` | `database/mcp_servers.rs` | MCP CRUD and Agent bindings |
| `app/admin/tool_allowlist.rs` | `database/tool_allowlist.rs` | Approval listing/mutation, reviewed observation read |
| `database/tool_security.rs` | `database/tool_allowlist.rs` (private query helper) | Move live `allows()` SQL behind a domain check, keep fail-closed dispatch semantics |
| `app/admin/speakers.rs` | `database/speakers/` | Speaker CRUD, voiceprints, catalog revision |
| `app/admin/speaker_policy.rs` | `database/speakers/policy.rs` | Agent Speaker policy, candidates and assignments |
| `app/admin/speaker_quick.rs` | `database/speakers/captures.rs` | Quick capture, promotion, voiceprint replacement |
| `session/speaker_observe.rs` | `database/speakers/policy.rs` or `database/speakers/observations.rs` | Policy/candidate snapshot SQL; retain scoring/inference in `session` |
| `app/admin/history.rs` | `database/history/queries.rs` | Bounded read, filter, sort and destructive purge SQL |
| `app/admin/deletion.rs` | `database/deletion.rs` | Reusable conditional delete and dependency guard |
| `app/admin/system.rs` | `database/providers.rs` / dedicated read method | Provider counts and loaded-vs-desired revision lookup |
| `app/admin/mod.rs` | `database/audit.rs` | Atomic admin audit writes and conflict audit |
| `database/mod.rs` | `database/providers.rs` for `enabled_provider_rows()` | Retain connection, migration and readiness ownership in the database root |

This is a **starting inventory**, not an assurance it is complete after newer commits. Generate a fresh full inventory in P0.

Useful reference paths:

- [`src/database/mod.rs`](../../crates/voice-agent-server/src/database/mod.rs)
- [`src/database/admission.rs`](../../crates/voice-agent-server/src/database/admission.rs)
- [`src/app/admin/agents.rs`](../../crates/voice-agent-server/src/app/admin/agents.rs)
- [`src/app/admin/deletion.rs`](../../crates/voice-agent-server/src/app/admin/deletion.rs)
- [`src/app/admin/speaker_quick.rs`](../../crates/voice-agent-server/src/app/admin/speaker_quick.rs)
- [`src/session/speaker_observe.rs`](../../crates/voice-agent-server/src/session/speaker_observe.rs)
- [`ADR-0073`](../adr/0073-required-database-and-device-admission.md)

## 2. Target module structure

The following is a **target map**, not a mandate to create empty or shallow files. Keep cohesive current implementations until an actual seam needs separating.

```text
crates/voice-agent-server/src/
  database/
    mod.rs                     # Database facade and intentional re-exports
    connection.rs              # OPTIONAL extraction: SQLite pool, migration, readiness
    error.rs                   # OPTIONAL extraction: shared low-level DB errors
    audit.rs                   # Typed write-audit and conflict-audit operations
    deletion.rs                # Conditional multi-resource deletion implementation
    agents.rs                  # Agent read/write
    templates/
      mod.rs                   # Template read/write
      relationships.rs         # Agent/Template and Template/Provider bindings
    devices.rs                 # Device read/write
    device_enrollments.rs      # Existing; add claim SQL from HTTP
    admission.rs               # Existing; retain snapshot semantics
    providers.rs               # Provider read/write and desired-state snapshots
    load_plan.rs               # Existing; retain required/optional/unbound behavior
    provider_config.rs         # Existing; only move if justified separately
    mcp_servers.rs             # MCP server settings and Agent bindings
    external_mcp.rs            # Existing admission snapshot
    external_mcp_policy.rs     # Existing network URL policy
    tool_allowlist.rs          # Observation, approval and dispatch permission SQL
    tool_security.rs           # Existing in-memory invalidation/security registry
    speakers/
      mod.rs                   # Speaker read/write facade and data records
      policy.rs                # Policy, grants, identification candidates
      voiceprints.rs           # Enrollment vector storage
      captures.rs              # Quick capture/promotion
    history.rs                 # Existing nonblocking archive writer + retention
    history/queries.rs         # OPTIONAL child query module, or a peer module
    secrets.rs                 # Existing secret reference handling
  app/admin/                   # HTTP-only parsing/validation/response mapping
  services/                    # Provider management and orchestration, not SQL
  session/                     # Voice processing and immutable session facts
```

**Rust module caveat:** If using both `database/history.rs` and `database/history/queries.rs`, declare `mod queries;` inside `history.rs`; do not simultaneously create `database/history/mod.rs`. Alternatively convert `history.rs` to `history/mod.rs` in a self-contained change. Do not move the archive writer merely because a directory is added.

### Ownership rules

- `database` owns SQL text, column selection, `FromRow` storage mappings, `QueryBuilder` composition, transactions, constraints, and persisted invariants.
- HTTP handler owns extracting payloads/headers, authentication, transport-specific validation, status codes, response JSON and ETag construction.
- Domain DB interface returns typed records/outcomes/errors, **never** `axum::Response`, `StatusCode`, or `Request`.
- Existing non-database validation can stay outside the SQL module when it is not a persisted invariant; keep one canonical implementation, not duplicated checks.
- Runtime orchestration remains in `services`/`session`: loading models, resolving secrets, inference, provider prewarm queues, scoring and websocket lifecycle are not database work.
- Changes requiring multiple tables in one atomic operation are implemented behind **one mutation interface** with one transaction. Do not concatenate separate auto-committing CRUD calls in the caller.
- `Database::pool()` may remain for integration fixtures during migration. Production code must stop calling it for queries; avoid an unnecessary breaking change for tests.

## 3. Deep-module interface design

Prefer a lightweight borrowed typed store sharing the existing `Database` pool. **No generic repository base class, ORM layer, async trait or dynamic dispatch is needed.** SQLite is the real local dependency: test the interface with actual migrated SQLite.

Illustrative shape (adapt exact types to the existing code; this is not a drop-in patch):

```rust
pub struct AgentStore<'a> {
    db: &'a Database,
}

impl Database {
    pub fn agents(&self) -> AgentStore<'_> {
        AgentStore { db: self }
    }
}

impl AgentStore<'_> {
    pub async fn get(&self, key: &str) -> Result<Option<AgentRecord>, DatabaseError> {
        // SELECT stays in database/agents.rs
        todo!()
    }

    pub async fn create(
        &self,
        input: NewAgent,
        audit: AuditContext,
    ) -> Result<AgentRecord, AgentWriteError> {
        // Begin transaction; INSERT; audit in same tx; COMMIT; return record
        todo!()
    }

    pub async fn update(
        &self,
        key: &str,
        expected_revision: i64,
        changes: AgentChanges,
        audit: AuditContext,
    ) -> Result<AgentWriteResult, AgentWriteError> {
        todo!()
    }
}
```

Avoid exposing `SqlitePool`, `SqliteRow`, `Transaction`, `QueryBuilder`, SQL snippets, table/column identifiers, or `impl Executor` through *public domain interfaces*. Internal helpers may take `&mut sqlx::Transaction<'_, sqlx::Sqlite>` when composing one transaction; keep them `pub(super)` or private. Existing database boot/config interfaces can retain SQLx-specific types when intentionally low-level.

### Domain input/output types

Use small named types where they reduce caller complexity:

- `NewAgent`, `AgentChanges`, `AgentRecord`, `AgentListFilter`, `AgentListPage`.
- `NewDevice`, `DeviceChanges`, `DeviceRecord`, `DeviceWriteOutcome`.
- `TemplateBindingChange` with `template_key`, `provider_type`, `provider_key`, `expected_revision`.
- `ProviderSnapshotScope` for current prewarm/diagnostics reads instead of exposing a generic SQL filter.
- `SpeakerCapturePromotion` for the complete reservation -> speaker -> voiceprint -> catalog -> audit transaction.
- `HistoryQuery` and `HistoryPurgeScope` with validated, typed allowlisted sort/filter values.

Do **not** introduce one type per SQL parameter if it only makes callers work harder. Group arguments when a method has multiple semantically coupled values or positional ambiguity.

### Error model and HTTP compatibility

A simple typed error taxonomy is useful, but the status/error-code mapping **must reproduce current behavior**:

- `NotFound` versus missing associated Agent/Template/Provider.
- `RevisionConflict` versus `InUse`/dependency conflict versus unique/resource conflict.
- `DatabaseBusy`, pool timeout, SQLite locked and general unavailable.
- Validation/policy failures that currently have a specific outward code.
- Cases where existing code deliberately fails closed rather than propagating a DB error (e.g. External MCP tool dispatch permission).

Define domain errors only for variants callers actually distinguish. A practical approach: `DatabaseError` for infrastructure failures plus domain mutation enums; central HTTP mapping converts domain results to the existing status and `error.code`, preserving `request_id` placement. **Before touching the mapper, snapshot current endpoint behavior in integration tests.** Do not collapse every write failure into `revision_conflict` or every query absence into `database_unavailable`.

`AuditContext` may carry request ID and trusted action metadata but must not take an Axum request or raw unvalidated resource/action strings from clients. Write events should not contain prompt text, bearer credentials, raw voiceprints, or provider secrets.

## 4. Non-negotiable invariants

### 4.1 Database startup and admission

- SQLite is a required startup dependency under accepted ADR-0073; migrations/schema checks run before publishing the listener.
- `is_reachable()` continues to run `SELECT 1` for readiness.
- Preserve WAL, foreign key, busy timeout and migration settings in `database/mod.rs`/`connection.rs`.
- Keep `admission.rs` as the snapshot seam: Device -> Agent -> Template -> Provider facts are resolved before WebSocket upgrade. Do **not** make the live actor re-read SQLite for every audio turn.
- Device disabled, Agent disabled, unknown/enrollment state, unavailable DB/runtime, and template fallback outcomes must retain their exact current semantics.
- Preserve the bounded template assignment graph and diagnostic data minimization.

### 4.2 Write transactions, revision and audit

- Agent/Device/Template/Provider/MCP/Speaker mutations use existing `revision`/`If-Match` optimistic-lock rules.
- The mutation, relationship updates and **success audit row** must commit atomically. An audit failure rolls back the mutation.
- Conflict audit behavior stays as-is (may be a separate non-mutating transaction/write); do not accidentally convert it to a successful mutation.
- Conditional delete must keep dependency checks in the transaction **and** the guarded `DELETE ... WHERE ... AND NOT EXISTS(...)`/revision protection; checking only before transaction is insufficient.
- Device creation must preserve: Agent enabled check, Template override resolution, enrollment cancellation, insert, audit, commit, then response.
- Template-provider binding must preserve: Provider type/enabled validation, binding upsert, Template revision change, audit, then post-commit prewarm.
- Post-commit runtime effects (Provider prewarm, relevant Speaker/session invalidations) must never fire for rolled-back writes.

### 4.3 External MCP authorization

- Do not mix Device tools with External MCP allowlist review; Device tools have distinct authorization semantics defined by current project policy.
- Keep reviewed External MCP observation contracts, fingerprints, observed server revision, per-Agent allowlist entries and sensitive/blocked flags.
- Maintain ordering of publication lock -> revision/observation checks -> transaction commit -> invalidation, including cancellation of stale pinned sessions.
- `ExternalToolGuard::allows()` currently makes a database-backed permission decision at **tool dispatch**, and rejects when the DB check fails. Moving its SQL into `database/tool_allowlist.rs` must preserve the dispatch-time revalidation and fail-closed behavior. Do not replace it with a permissive cached Boolean.
- Discovery/network operations stay outside database transactions.
- Never log or return resolved secret values; moving code must not change the credentials/secrets policy. Any removal of database-stored API keys or credential references is a **separate, explicitly scoped change**, not part of this refactor.

### 4.4 Speaker and voiceprints

- Speaker policy, candidate selection and catalog revision come from database snapshots, but inference, embedding scoring and voice processing remain outside database.
- An invalid persisted Speaker policy must not silently become `off` where existing admission treats it as an error.
- Preserve embedding space/dimensions validation, revision handling and catalog version publication.
- Quick enrollment promotion is one atomic operation: accepted capture reservation, Speaker row insert, voiceprint insert, capture committed/tombstone update, catalog revision, audit, commit.
- Preserve runtime/embedding-space compatibility checks; invalidate affected sessions only when the write commits.
- Do not introduce SQL queries inside the PCM/VAD/ASR/TTS path.

### 4.5 Transcript archive and retention

- Capture remains optional; retention does not depend on capture being enabled.
- Live-session archive handoff is non-blocking `try_send`, not `await` on SQLite.
- History is archival, not conversational authority; no history query feeds the realtime Dialogue History.
- Preserve text size/role/sequence bounds, identity attribution, UTC timestamps and retention interval.
- Admin list filters, fixed sort allowlists, pagination and scoped destructive purge remain unchanged; invalid filters must never widen a query to all rows.
- The explicit confirmation for `all` history purge remains mandatory; audit count and transaction stay correct.

### 4.6 Performance and secrets

- One shared SQLite connection pool, not one pool per domain.
- Do not extend write transaction lifetime across network calls or model loading.
- Preserve bounded reads, dynamic `ORDER BY` allowlists, `QueryBuilder` binding and prewarm coalescing behavior.
- Preserve snapshot privacy (no secret value, prompt or raw voice embedding in logs).
- No schema migration is needed merely to move query code. Do not edit migrations or their checksums for this refactor.

## 5. Step-by-step implementation plan

Each phase should be independently compilable, tested and reviewable. Prefer one domain per commit/PR. Do not leave a long-lived duplicate SQL path or compatibility forwarding layer after its callers have migrated.

### P0: Baseline, inventory and contract tests (required first)

1. Work from a fresh branch based on the agreed base ref. Record base commit SHA.
2. Inventory **all** production SQL execution sites, distinguishing test-only code, type imports/config parsing and migrations from queries.
3. Create an inventory table: source path + function + tables touched + transaction scope + expected errors + side effects + destination owner + tests.
4. Run existing targeted tests and capture baseline failures before edits.
5. Add characterization tests for endpoint behavior before moving handler logic (revision, in-use, busy/unavailable mapping, audit, ordering and empty results).
6. Identify concurrent mutation and security races that require an end-to-end test.

Suggested commands (from repository root):

```bash
rg -n 'sqlx::(query|query_as|query_scalar|raw_sql|QueryBuilder)' crates/voice-agent-server/src
rg -n '\.pool\(\)|\.begin\(\)|sqlx::Transaction' crates/voice-agent-server/src
rg -n 'sqlx::(query|query_as|query_scalar|raw_sql|QueryBuilder)' crates/voice-agent-server/tests
cargo fmt --all -- --check
cargo check -p voice-agent-server
cargo test -p voice-agent-server --test admin_api
cargo test -p voice-agent-server --test device_admission
```

The inventory is more authoritative than a text grep; `sqlx::query!`, imported macros, aliasing, and indirect query builders may require additional review. Exclude test modules under `src` from the **production** SQL rule using module-context inspection; do not blanket-exempt an entire production file just because it contains `#[cfg(test)]`.

### P1: Establish minimal DB interfaces and migrate Agents/Providers

**Agents:**

1. Add `database/agents.rs`, register its module, and expose `Database::agents()` (or a small direct method group if that is shallower).
2. Move Agent SQL, typed row mapping, filter/order allowlists, transaction/revision handling and audit write into this module.
3. Keep `app/admin/agents.rs` limited to transport validation/response mapping.
4. Verify Agent CRUD and conflict audit are identical to baseline.

**Providers:**

1. Add `database/providers.rs`; move desired provider reads from `database/mod.rs` without breaking existing snapshot consumers.
2. Move Provider CRUD, relation counts, adapter lookup, diagnostics bounded fetch, prewarm selection and startup/provider overview SELECTs.
3. Leave registry materialization, external secret resolution, diagnostics execution and coalesced prewarm queue in `services`.
4. Prefer a single typed `DesiredProvider` representation shared by existing load-plan/admission/prewarm paths, not parallel DB-only and runtime-only copies unless they encode meaningfully distinct states.
5. Preserve required/optional/unbound planning. Keep `database/load_plan.rs` independent from runtime construction.
6. Verify Provider update revision, post-commit prewarm and stale runtime behavior.

**Exit gate:** No production SQL in `app/admin/agents.rs`, `app/admin/providers.rs`, `app/admin/provider_adapters.rs`, `services/provider_diagnostic.rs`, `services/provider_prewarm.rs`, or the affected startup path; tested HTTP results unchanged.

### P2: Templates, Devices and enrollment claims

1. Migrate Template CRUD and Template-Provider/Agent relationships, keeping any multi-table mutation atomic.
2. Migrate Device CRUD into `database/devices.rs`. Do not split Agent lookup, Template override, enrollment cancellation, audit and insert into separately committed calls.
3. Move admin enrollment claim SQL into existing `database/device_enrollments.rs`; reuse existing enrollment-state rules rather than creating another enrollment implementation.
4. Keep `database/admission.rs` stable unless a specific duplicated lookup can be removed without changing its immutable snapshot contract.
5. Run websocket admission, enrollment, session profile and Admin tests.

**Exit gate:** No production SQL in `app/admin/templates*.rs`, `app/admin/templates/relationships.rs`, `app/admin/devices.rs` and `app/admin/enrollments.rs`; same conflict/result codes and WS admission decisions.

### P3: Speakers and History

1. Migrate Speaker CRUD, policy and candidate SQL; retain voiceprint/embed-scoring separation.
2. Extract the `session/speaker_observe.rs` SQL readers into Speaker DB interfaces; preserve frozen plan interpretation and invalid-policy outcome.
3. Migrate quick-capture promotion/replacement as one transaction each; retain precise reservation/tombstone behavior.
4. Move Admin History SQL (read/filter/purge) to `database/history/queries.rs` or another cohesive private module; keep writer/retention orchestration intact.
5. Run Speaker recognition, admin, archive and lifecycle tests.

**Exit gate:** No production SQL under `app/admin/speaker*.rs`, `app/admin/history.rs`, or `session/speaker_observe.rs`. No SQL added to real-time inference paths.

### P4: MCP and authorization (security-gated)

1. Migrate MCP server CRUD/Agent bindings into `database/mcp_servers.rs`; preserve URL/header/secret-reference semantics in their existing validator modules.
2. Move Admin External MCP list/review/approval SQL into existing `database/tool_allowlist.rs`; retain transaction, observation and fingerprint rules.
3. Move the SQL statement used by `ExternalToolGuard::allows()` to an internal typed database check **without** weakening live dispatch-time revalidation.
4. Keep the publication lock, invalidation and catalog revision check at the original security-critical sequencing seam; if changing their owner, add tests proving concurrency ordering, not just happy-path permission.
5. Run MCP admission and tool-round tests, including changed fingerprint, revoked grant, blocked observation, disabled binding, DB failure, and already-admitted session cases.

**Exit gate:** Admin MCP handlers have no SQL; stale/unauthorized External MCP tools are never executed.

### P5: Shared deletion, audit and enforcement

1. Move `app/admin/deletion.rs` SQL and its reusable `DeleteSpec` implementation to `database/deletion.rs`. Keep deletion dependency/guarded SQL centralized. HTTP deletion handlers become thin wrappers.
2. Move `audit()`/`audit_conflict_action()` SQL from `app/admin/mod.rs` to `database/audit.rs`. Preserve both success and conflict audit semantics.
3. Simplify re-exports and module visibility. Keep existing public methods only where they prevent test or intentionally supported code churn; remove redundant SQL aliases or forwarding wrappers.
4. Add an enforceable static/CI check limiting production **query execution** to `src/database/`, with intentional allowances for tests/fixtures and SQLx connection/config parsing.
5. Re-run the complete test matrix, review SQL inventory, update architecture docs and verify no API/web changes are necessary.

**Exit gate:** All applicable production queries are owned under `database/`; no duplicate implementations or new abstract frameworks; full relevant test suite passes.

## 6. Transaction and side-effect pattern

Maintain the existing ordering:

```text
HTTP handler
  -> parse/auth/validate input and expected If-Match
  -> database.domain().mutate(command, audit_context)
       -> acquire required publication lock where appropriate
       -> begin SQLite transaction
       -> check current resource and revision / dependency invariants
       -> write rows
       -> write success audit row in same transaction
       -> commit
       -> invalidate security-pinned sessions if required (after commit)
       -> return typed mutation result
  -> service-level post-commit action (e.g. Provider prewarm)
  -> serialize existing HTTP status, body and ETag
```

For mutations with no security publication lock, omit that step. Some current handlers acquire publication protection across read/transaction/commit/invalidation; preserve their exact coverage. Do **not** move a lock after the write or release it between commit and required invalidation if that creates an authorization race.

Where multiple domain tables participate in one business operation, let its **owning domain interface** orchestrate private SQL helpers in one transaction. Example: `DeviceStore::create()` can call private Agent/Template lookup helpers on the same transaction, rather than exposing transaction ownership to the Admin handler. If a helper needs direct SQLx transaction access, that is an **internal seam**, not an external one.

A post-commit notification failure must not retroactively report an already committed database mutation as rolled back; follow the current endpoint contract and recovery strategy, without inventing a distributed transaction.

## 7. Required regression coverage

| Area | Minimum tests before accepting a phase |
| --- | --- |
| Agents | Create/get/list/update/delete; bad input; unique key; revision mismatch; in-use; audit in same commit |
| Templates | Assign/unassign; default selection; Provider binding type/enabled checks; revision; disabled assignment semantics |
| Devices/enrollment | Enabled Agent; valid Template override; duplicate ID; pending enrollment cancellation; claim concurrency; unknown/blocked/registered paths |
| Admission | Pre-upgrade snapshot; disabled Agent/Device; missing/unavailable DB; no unexpected runtime SQL; unchanged fallback |
| Providers | Required/optional/unbound; enabled filtering; diagnostics snapshot bounds; prewarm after commit; stale revisions |
| MCP review | Server version drift; changed tool fingerprint; sensitive/blocked flags; revocation; approval; fail-closed dispatch |
| Speakers | Policy `off`/`observe`/invalid; candidate set; vector space/dims; quick capture reservation; catalog revision; post-commit invalidation |
| History | Filters and ordering; invalid filter rejection; purge confirmation; audit; retention independent of capture; writer never blocks a turn |
| Deletion/audit | Dependency guard race; correct `in_use` and revision errors; no orphaned rows; conflict audit outcomes |
| Database | WAL/foreign keys/migration compatibility/readiness; busy/locked/pool timeout mapping |

Existing relevant test files (run those still present on the implementation branch):

```bash
cargo test -p voice-agent-server --test admin_api
cargo test -p voice-agent-server --test device_admission
cargo test -p voice-agent-server --test device_enrollment_ws
cargo test -p voice-agent-server --test provider_load_plan
cargo test -p voice-agent-server --test provider_admission_snapshot
cargo test -p voice-agent-server --test external_mcp_admission
cargo test -p voice-agent-server --test tool_round_executor
cargo test -p voice-agent-server --test speaker_identification_builtin
cargo test -p voice-agent-server --test transcript_archive
cargo test -p voice-agent-server --test database_bootstrap
cargo test -p voice-agent-server --test lifecycle
cargo test -p voice-agent-server
```

Some suites may depend on optional model assets, external tools or local test environment. Report `PASS`, `FAIL` or `UNAVAILABLE` per command with a cause; do not claim a phase is verified by compilation alone. Use the repository's existing deterministic qualification/test infrastructure where applicable, not real credentials.

### SQL placement check

After migration, audit each production file outside `src/database/` for:

```bash
rg -n 'sqlx::(query|query_as|query_scalar|query!|query_as!|query_scalar!|raw_sql|QueryBuilder)' \
  crates/voice-agent-server/src/app \
  crates/voice-agent-server/src/services \
  crates/voice-agent-server/src/session
```

Any hit in **compiled production code** must be eliminated or documented as a deliberately approved exception. SQLx imports used solely for a type, test fixture or configuration parsing are not query-execution violations. Also search for `SqlitePool`/`.pool()`/`.begin()` to find direct access that avoids qualified `sqlx::` calls. Favor a simple, inspectable check over a custom AST framework unless text-based enforcement proves inadequate.

## 8. Review rules / anti-patterns

Reject an implementation that:

- Adds `Repository<T>`, `DbService`, `GenericDao`, macros for boilerplate CRUD or traits with only one actual adapter.
- Moves a `SELECT` into a helper but leaves the calling HTTP handler responsible for the same transaction/SQL semantics, making the helper shallow.
- Replaces one atomic mutation with two independent commits.
- Converts `sqlx::Error` to a single ambiguous `bool`/`Option` and changes outward errors.
- Duplicates domain SQL in both `database/` and Admin/Session for convenience.
- Stores the same provider snapshot in multiple incompatible structures without reason.
- Starts or awaits network calls inside SQLite write transactions.
- Uses unsafe dynamic query strings or unvalidated client-supplied SQL identifiers.
- Drops fingerprint/revision checks or caches permission as a permanent allow decision.
- Adds a new pool for each domain, new migration, web endpoint or UI change as part of this task.
- Rewrites existing deep modules solely for visual consistency.

**Deletion test:** if the new DB module were removed, would real SQL/error/transaction complexity return to many callers? If not, the module is probably unnecessary forwarding code. Prefer deeper domain methods with fewer public entry points.

## 9. Task checklist for the implementation agent

- [ ] Record base commit, inventory every production SQL site and baseline test outcomes.
- [ ] Confirm existing ADR and HTTP behavior before changing interfaces.
- [ ] Introduce shared DB error mapping and audit context only to the extent required.
- [ ] Move Agent SQL and preserve revision/audit behavior.
- [ ] Move Provider SQL, prewarm/diagnostic/startup reads and preserve runtime semantics.
- [ ] Move Template relationship and Device/enrollment SQL, preserving atomicity.
- [ ] Move Speaker/voiceprint/policy/quick capture SQL, preserving snapshots and security.
- [ ] Move History read/purge SQL while preserving nonblocking archive writer.
- [ ] Move MCP and External allowlist SQL, prove fail-closed dispatch and invalidation.
- [ ] Move shared deletion and audit SQL, preserving dependent-resource guard rules.
- [ ] Remove duplicate SQL, minimize visibility, and enforce production query placement.
- [ ] Run `cargo fmt --all -- --check`, `cargo check -p voice-agent-server` and applicable tests.
- [ ] Update relevant `docs/` architecture/flow records when interface locations change.
- [ ] Produce an implementation report: changed file inventory, before/after query locations, contracts preserved, tests and exceptions.

## 10. Scope and acceptance criteria

**In scope:** restructure production SQLx access into cohesive database domain modules; migrate callers; encode transaction and persisted invariants at the module interface; improve consistency of DB errors; add regression and placement checks; update corresponding docs.

**Out of scope:** schema or migration changes, adding PostgreSQL/MySQL, replacing SQLx, restructuring web routes/UI, modifying API credentials storage, changing Provider Load Plan logic, changing Voice Session or Speaker authorization policy, optimizing SQL itself unless a behavior-preserving change is needed for the migration.

The work is complete only when:

1. Each production DB query has one explicit domain owner under `src/database/`.
2. Admin, services and session have no direct production SQL query execution.
3. HTTP contract, audit, transaction, optimistic locking and session invalidation behavior remain equivalent.
4. WebSocket admission, Provider snapshots, Speaker identification and MCP authorization are behaviorally unchanged.
5. Targeted and full feasible test gates pass, with environment-limited tests explicitly reported.
6. No new schema migration or unnecessary repository abstraction was introduced.

**Suggested delivery:** one implementation branch, reviewable commits/PRs by domain, with P0 characterization tests first and security-critical MCP migration last. Do not push or merge without explicit repository workflow authorization.
