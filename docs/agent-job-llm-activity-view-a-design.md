# Agent Job — LLM Activity / View A

> **Status:** Design v1.1 — revised after PONYTAIL Review, 2026-10-14.  
> **Baseline:** `main`; implementation branch: `feature/job-llm-activity-view-a`.  
> **Approved visual source:** `job-llm-activity-3-views.html`, **View A / Timeline First**, from the approved UI prototype shared in the conversation.  
> **Scope:** Approved specification and implementation record for the feature branch.

## 1. Binding decisions

1. **UI fidelity overrides PONYTAIL reductions.** The approved **View A** is the target, **not** a generic accordion list: retain the existing Voice Agent Studio shell, Job Detail header, **four summary cards**, **left execution timeline**, and the **right-hand Selected LLM Run + Execution Progress panels**. Run card selection synchronizes the persistent inspector; clicking an already-selected card toggles its inline stages.
2. **LLM activity starts at reservation:** create a durable `llm_run_started` before provider acquisition, atomically with the existing model-call budget reservation. Provider failure/timeout must still leave a visible Run.
3. **Historical identity is exact:** initially display the pinned Provider ref/source; after validation/acquisition, display the **actual adapter, model and transport** resolved for that Run. Never resolve historical model identity from mutable current settings.
4. **Backend SQL ownership is mandatory:** **all SQL queries/transactions for this feature, including Provider lookups and Background Settings authorization, live exclusively under `crates/voice-agent-server/src/database/`.** The orchestration, runtime, provider-adapter, and Admin handler Modules call typed Database Interfaces. No `sqlx::query*`, raw `SELECT/INSERT/UPDATE/DELETE`, `pool().acquire()` or `BEGIN` inside those logic files. SQLx migrations live in the existing `crates/voice-agent-server/migrations/` directory as the sole permitted schema-DDL exception.
5. **PONYTAIL simplification:** reduce redundant lifecycle events and pass-through abstractions, and reuse the existing `job_events`, `JobChangeFeed`, Admin SSE and database access patterns. Do **not** delete/alter the accepted View A visual elements.
6. Rust `PlanCompiler`, the scheduler, quota, pin/authority checks, Step checkpoints, unknown-outcome rules and Voice LLM transport remain authoritative and unchanged in behavior.

### Non-goals

No full prompts/responses, hidden reasoning text, credentials, plugin stdout, token-stream rendering, vendor-background polling, OpenTelemetry, Redis, new public endpoint, Agent enable toggle, new top-level Logs page, or second `job_llm_calls` table. Responses remains one-shot (`store:false`, `background:false`, `stream:false`), per ADR 0099.

## 2. Verified main baseline and implications

| Location on `main` | Current behavior | Impact of the design |
|---|---|---|
| `apps/admin-web/src/pages/jobs/JobDetailPage.vue` | `DetailHeader` + two columns; Steps/Artifacts/Result left, Execution Context + Event Timeline right; SSE reconciles authoritative reads. | **Replace the primary two-column content arrangement with the approved View A**; preserve existing Step/Artifact/Result actions in lower secondary sections so no capability is lost. |
| `apps/admin-web/src/style.css` | `studio-panel`, semantic Tailwind tokens, light/dark palettes. | Map approved mock surfaces to existing tokens, no hard-coded theme or new design system. |
| `apps/admin-web/src/api/types/agent-jobs.ts` | `JobEventType` has six types. | Extend the union with metadata-only LLM event types, keep REST envelope. |
| `apps/admin-web/src/api/agent-jobs.ts` / `api/job-updates.ts` | Cursor-paged events + authenticated SSE invalidations. | Reuse unchanged transport; project Run cards from the same events. |
| `database/jobs/events.rs` and migration `0107_job_execution_events.sql` | Append-only event sequence per Job, max 4096-byte JSON. | New forward migration widens event CHECK; no mutation/backfill of old data. |
| `database/jobs/orchestration.rs::reserve_model_call` | One `BEGIN IMMEDIATE` handles budget and `plan_attempt`. | Add `llm_run_started` to **that same transaction**, not a second post-reservation write. |
| `services/background/orchestration.rs` | Planner/Repair/Replan use `propose()`; Worker validates JSON and checkpoints. | Supply `run_id` and record validation outcome without changing executable Plan control flow. |
| `services/background/models.rs` | Acquires pinned provider and capacity; checks auth and settings; calls `background_complete`. **Contains direct SQL/provider-row access today.** | **Move SQL into `database/`**; emit safe lifecycle events at existing steps. |
| `services/background/providers.rs` | `pin()` and `options()` query `providers` via SQLx outside `database/`. | **Move their DB implementation into `database/jobs/providers.rs`**; update all callers in `engine.rs`/`models.rs`/DB. Delete the old DB-querying implementation after migration. |
| `services/background/engine.rs` | Opens raw DB connections and calls `providers::pin/options`. | Use typed Database Interfaces rather than handling `SqliteConnection` in logic. |
| `providers/llm/openai/responses.rs` | Actual usage currently available only via `tracing`; return type is `String`. | Return optional typed Background usage metadata; Voice `stream()` untouched. |
| `database/jobs/recovery.rs` | Terminalizes interrupted Jobs and emits `job_finished`. | No new recovery query for v1; UI derives interrupted Run from existing Job terminal event. |
| `services/background/artifacts.rs` | Retention already deletes `job_events` before Job deletion. | Reuse same retention; no new GC. |

### Architecture rule for SQL (explicit acceptance gate)

**Allowed runtime query sites:** `crates/voice-agent-server/src/database/**`, including `database/jobs/events.rs`, `database/jobs/orchestration.rs`, and new `database/jobs/providers.rs` (or the repository's equivalent existing DB submodule). Schema DDL: `crates/voice-agent-server/migrations/**`.

**Not allowed:** any SQL literal, `sqlx::query`/`query_as`/`query_scalar`, DB pool borrow, `SqliteConnection` or transaction acquisition inside:
- `services/background/orchestration.rs`
- `services/background/models.rs`
- `services/background/engine.rs`
- `services/background/providers.rs` (should become a pure typed adapter or be removed)
- `providers/llm/*`
- `app/admin/background.rs`

**Specific main debt to relocate while implementing:** provider `SELECT ... FROM providers` in `services/background/models.rs::acquire`; `SELECT enabled FROM background_settings` and direct requester authorization in `complete_inner`; `providers::pin/options` from `services/background/providers.rs`; `engine.rs`'s connection acquisition and pin resolution. Keep only business orchestration, provider runtime leases, timeouts, capacity permits and cancellation in `services/background/*`. Do not perform a repository-wide unrelated SQL migration as part of this feature.

```text
Admin Job Detail / SSE ──> JobEngine / orchestration ──> Database (typed Interface)
                                        │                  ├─ jobs/orchestration.rs
                                        │                  ├─ jobs/events.rs
                                        │                  └─ jobs/providers.rs
                                        │                        └─ SQLx queries and transactions
                                        └── ManagedModels ──> runtime.background_complete()
                                             │
                                             └── Database::... (no direct SQL in models.rs)
```

**Provider safety:** preserve exactly pinned `ProviderPin` (ref, incarnation, revision, digest), config/database origin, deployment snapshot behavior, reauthorization immediately before dispatch, publication guard lifetime, deadline/cancellation, permit quarantine and job status checks. Extracting SQL must not remove or reorder these enforcement points. The Database Interface should provide the needed **typed resolved snapshot** and **pre-dispatch authorization/pin check**; the caller still owns provider-runtime admission and cancellations.

## 3. Approved View A — precise web UI contract

The approved `job-llm-activity-3-views.html` is the **visual acceptance baseline**. Implement its `renderA()` composition and interaction, adapted to the production Admin layout, i18n, accessibility and real data. Ignore the **prototype-only View A/B/C switcher, banner, "Design intent" explanatory card, mocked names/values and artificial progress** in production.

### 3.1 Full page hierarchy — DO NOT restructure into a modal inspector

**Preserve from approved prototype:**
1. Left Studio navigation: existing `Overview / Agents / Devices / Providers / Background / Jobs / Plugin Store`. `Jobs` is active. In production reuse the real Admin shell/sidebar.
2. Workspace breadcrumb/topbar (or the existing shell's equivalent).
3. Compact Job Detail header: **Back to Jobs**, Goal/Job title, Job ID, creation/requester metadata, status badge and **Refresh/Cancel** or **Retry** actions according to current Job status. Preserve `DetailHeader` behavior on `main`.
4. **Four summary KPI cards in a row**, exactly the approved information hierarchy:
   - **LLM Runs** — total Run count; current running/finished subtitle.
   - **Plan Repair** — repair count; accepted attempt/status subtitle.
   - **Observed Tokens** — sum only **known** usage, marked partial when some Providers report none; do not invent totals.
   - **Elapsed Time** — Job elapsed time, progressing while active, frozen when terminal.
5. **Two-column View A body**: left **Execution Timeline**, right stacked **Selected LLM Run** and **Execution Progress** cards.
6. Keep prior Step / Artifact / Result UI **below** the approved View A body, as a secondary information area; they must not replace a View A right panel or interrupt its primary timeline. Existing downloads, links and result formatting still work.
7. Old six-type raw Execution Timeline/Context, if retained for legacy/debugging, belongs in a **collapsed secondary section below View A**, not in the approved right-hand inspector slot. Do not display the same LLM event twice by default.

**Explicitly reject these prior-document suggestions:** "4 KPI deletion", "Inspector as popup/drawer", "All/Planner/Worker/Errors filter bar", "All/Errors filter", "auto-expand newest active Run", "permanent three-column split", or "Execution Context/legacy Event Timeline occupying View A's right column". These would deviate from the approved HTML.

### 3.2 View A desktop structure

```text
┌────────────────────────────────────────────────────────────────────────────────────────────┐
│ Voice Agent Studio sidebar │ Breadcrumbs: Workspace / Background / Jobs / Job detail     │
│                           ├───────────────────────────────────────────────────────────────┤
│ Jobs (active)             │ ← Back to Jobs   Job goal / ID       Running  Refresh  Cancel │
│                           ├───────────────────────────────────────────────────────────────┤
│                           │ [LLM RUNS] [PLAN REPAIR] [OBSERVED TOKENS] [ELAPSED TIME]     │
│                           ├────────────────────────────────────┬──────────────────────────┤
│                           │ VIEW A · TIMELINE FIRST             │                          │
│                           │ LLM activity, in execution order    │                          │
│                           │                                    │                          │
│                           │ Execution timeline       3 runs    │ Selected LLM Run         │
│                           │                                    │ Read-only                │
│                           │ ├ 09:42  Run 01 · Planner  Failed   │ Provider | Model         │
│                           │ │  title, provider, model, transport│ Transport | Source       │
│                           │ │  ▾ expanded lifecycle stages     │ LLM Role | Run ID       │
│                           │ │  attempt / Collapse              │ ─────────────────────   │
│                           │ ├ 09:42  Run 02 · Plan Repair ✓     │ Output & metrics         │
│                           │ │  provider, model, transport      │ Duration / Token usage  │
│                           │ └ 09:42  Run 03 · Worker  Running   ├──────────────────────────┤
│                           │    provider, model, transport      │ Execution progress       │
│                           │                                    │ Progress bar             │
│                           │                                    │ Planner & Repair ✓       │
│                           │                                    │ Worker synthesis ◉       │
│                           │                                    │ Job checkpoint ○         │
│                           ├────────────────────────────────────┴──────────────────────────┤
│                           │ [Existing Steps / Artifacts / Result, secondary below]         │
└────────────────────────────────────────────────────────────────────────────────────────────┘
```

The order, card placement and role of each section **must** match the approved View A. This wireframe is a textual mapping of that HTML, not a new alternative.

### 3.3 Timeline card, selection and Inspector — actual prototype interactions

The approved View A uses `state.selected` and `state.expanded`:
- Each Run card: **time rail with status circle**, Run number + role, status pill, work title, inline provider badge + model + transport, and attempt/Step footer.
- Selecting an **unselected** card: selects it, **expands its lifecycle stages** and updates the **Selected LLM Run panel on the right** immediately. Previously expanded cards may remain expanded, matching the prototype.
- Clicking the **already-selected** card: toggles its inline stages between **View stages** and **Collapse**, keeping it selected in the right inspector.
- Stage rows use check/error/pending icons, a stage label and safe duration/error path. Failure highlights the diagnostic; running state highlights the active stage.
- Right **Selected LLM Run** card is **always visible at desktop widths**, not a modal. Two-column property grid: Provider, Model, Transport, Source, LLM Role, Run ID. Beneath: Output & Metrics showing Duration and Token Usage. **Read-only** label.
- Right **Execution Progress** card: progress bar and three rows **Planner & Repair**, **Worker synthesis**, **Job checkpoint**. Calculate from actual committed Job Plan/Steps/Events, not the mock's "2 of 3 stages" text. For Jobs without Worker Steps, describe real phase; do not fabricate completed stages.
- Initial selection: first Run when available (prototype initializes `llm_01` expanded). When a new Run arrives, preserve current selection and expansion; if no Run existed, select and expand the first.
- All visible data comes from real per-Job events; no demo placeholders or fake runtime activity in production.

### 3.4 Styling and responsive rules

| Approved prototype visual | Production equivalent |
|---|---|
| White rounded panels, subtle border/shadow | Existing `studio-panel`, `bg-card`, `border-border`, semantic elevation |
| Violet selected Run, provider pill | Existing `studio-violet`/`primary` tokens, current `Badge` variants |
| Cyan running pulse, green complete, red error | Existing semantic `studio-cyan` / `success` / `danger`; respect reduced motion |
| Two-column timeline / stacked Inspector | Existing responsive grid conventions, left wider than right; **do not** add a third desktop column |
| Four KPI row | Compact cards on desktop, responsive 2x2 or stacked on small screens |
| Right Inspector | Below Timeline on narrow screens with the same fields and stage selection; no new overlay required |
| Sidebar/topbar/header | Real shared Admin shell + `DetailHeader`; do not duplicate the shell within `JobDetailPage.vue` |

Accessibility: each card selection is an actual keyboard-operable button/summary, accessible state shown with `aria-expanded`/`aria-current` (as appropriate); no nested buttons. Reuse existing `Badge`, `Button`, CSS tokens and VI/EN i18n. Render ID/model/diagnostics as escaped Vue text, no `v-html`. The mock's View switcher is **not** part of production.

### 3.5 Empty, error and historical data

- Job `queued`: show real status; zero Runs; **do not synthesize `llm_run_started`**.
- `planning` with no model reservation yet: pending state until actual first Run.
- Run started but Provider not acquired: show **pinned ref and source**, "Resolving model" or "—" for unknown adapter/model/transport.
- Provider failure, capacity timeout or refusal: show the Run and the **last verified stage only**, plus bounded error.
- Responses in-flight: "Waiting for response", no simulated tokens (Responses `stream:false`).
- Repaired Plan: distinct Planner and Plan Repair Runs. A rejected proposal is **not** a completed Job.
- Terminal Job with Run missing terminal event: display "Interrupted / unconfirmed" using existing `job_finished`/Job status; never mark Run completed.
- Legacy Jobs with only old events: four KPIs use supported values, timeline shows **Detailed LLM activity unavailable for this Job**, and old Steps/Events stay readable.
- SSE loss/reconnect, server restart or route change: reload authoritative events, deduplicate by `sequence` and keep the selected Run where possible.

## 4. Minimal durable LLM event contract (PONYTAIL)

Keep `job_events`. The original six types stay unchanged: `plan_attempt`, `plan_rejected`, `plan_accepted`, `step_started`, `step_finished`, `job_finished`.

**Five additive event types** are sufficient for the approved UI:

| Type | When and mandatory payload | Optional safe metadata |
|---|---|---|
| `llm_run_started` | **Same transaction as budget reservation**; `run_id`, `role` (planner/worker), `attempt_no`, `provider_ref`, `provider_source` | `step_key`, `plan_revision`, `phase` (initial/repair/replan), `pin_revision`/`pin_incarnation` |
| `llm_provider_acquired` | Exact Provider pin verified/leased; `run_id`, `adapter`, `model`, `transport` | `provider_wait_ms` |
| `llm_request_started` | Provider background invocation initiated immediately before `runtime.background_complete()`; `run_id` | `capacity_wait_ms`, `message_count`, `input_bytes` |
| `llm_response_received` | Successful provider text response received; `run_id` | `request_duration_ms`, `output_bytes`, optional `input_tokens`/`output_tokens`/`reasoning_tokens`/`cached_input_tokens` |
| `llm_run_finished` | Invocation outcome after Rust validation or error; `run_id`, `status` (completed/failed/cancelled) | `error_code`, `duration_ms`, `validation_code`, `validation_path`, `validation_message`, `capacity_wait_ms` (failure before dispatch) |

### Deliberate simplifications

- No separate `llm_capacity_acquired` event: `llm_request_started.capacity_wait_ms` describes successful capacity admission; if it times out, `llm_run_finished` records the failure and wait duration.
- No separate `llm_validation_finished` event: Rust emits `plan_rejected`, `plan_accepted` or `step_finished` at domain checkpoints; `llm_run_finished` carries safe validation diagnostics. UI can show "JSON validation passed / Plan rejected" as **derived labels** from these events, not a seventh durable event.
- Two `role` values (`planner` and `worker`); display **Plan Repair** and **Replan** from explicit `phase`/`attempt_no`/`plan_revision`, never from guesswork. Preserve accepted domain meanings.
- For `llm_request_started`, UI keeps the **approved text** "Request dispatched" as shorthand for **the Provider background call being initiated**. It is **not** proof the upstream server received or processed the HTTP request. This stage can only be recorded after the existing admission/authorization checks, never when only an orchestration-layer method was entered.
- `llm_response_received` means accepted **provider completion text** was obtained. Rust Plan compilation still decides acceptance; a rejected Plan shows a failure stage even when the Provider returned successfully.
- Missing usage is `null`/not available, never fabricated zero; only aggregate observed tokens with coverage context.
- Use opaque UUID `run_id`, not `Job ID` or Step attempt ID, as per-call correlation. If possible include `run_id` in `plan_rejected`/`plan_attempt` payloads; do not rewrite historical rows.

### Example — actual REST contract

`GET /api/admin/jobs/{id}/events?after=SEQ&limit=100` remains the only event read route.

```json
{
  "sequence": 12,
  "event_type": "llm_run_started",
  "created_at": "2026-10-14T02:42:01Z",
  "step_key": null,
  "payload": {
    "run_id": "19a37f79-18aa-422f-9f2f-e0f3a930e1ef",
    "role": "planner",
    "phase": "initial",
    "attempt_no": 1,
    "provider_ref": "config:llm:openai-primary",
    "provider_source": "config",
    "pin_revision": 4
  }
}
```

Per-Job `sequence` is the ordering authority; existing `created_at` has **second** precision. Use monotonic timers for measured milliseconds and avoid claims that durable event timestamps are millisecond precise.

### Write rules / migration

1. Create a **new forward SQLx migration** rebuilding the current `job_events` CHECK with these five extra values. Preserve existing rows, `PRIMARY KEY(job_id, sequence)`, Job FK, 4096-byte JSON cap and backward-compatible read payload. **Do not edit migration `0107`.**
2. At `reserve_model_call` inside `database/jobs/orchestration.rs`: quota check + `budget_used_json` update + `llm_run_started` + existing Planner `plan_attempt` share **one** `BEGIN IMMEDIATE` transaction. On failure, no budget increment and no phantom Run.
3. Other activity appends use bounded SQLx writes **only in `database/jobs/events.rs`**, short transactions, and `job_changes.publish_committed(job_id)` only **after commit**. No transaction spans provider awaits.
4. Safe metadata only; caps: `model`/`provider_ref` <=128 bytes, diagnostic/path <=256 bytes; never persist raw request/response, headers, token strings, API key, plugin output, source text or hidden reasoning.
5. A telemetry write failure must not make the executor retry an ambiguous LLM call. Treat progress telemetry separately from authoritative Step/Job checkpoint: log write failure to server tracing, preserve real execution result and current quarantine policy. Commit related terminal domain checkpoint and its event atomically where relevant.
6. Existing Job retention already removes `job_events`; no second garbage collector.

## 5. Backend Module/Interface design with strict SQL locality

### 5.1 Single Database owner — no new pass-through JobLlmActivity Module

Prefer the existing `Database` and `database/jobs/events.rs` instead of introducing `services/background/llm_activity.rs` just to forward writes. The Interface is small, typed and testable:

```rust
// Illustrative Interface; refine names/types to match existing code.
struct LlmRunContext {
    run_id: Uuid,
    job_id: String,
    step_key: Option<String>,
    role: LlmRole,    // Planner | Worker
    phase: LlmPhase,  // Initial | Repair | Replan | Step
    attempt_no: u32,
}

impl Database {
    // Contains existing budget transaction and appends llm_run_started.
    async fn reserve_model_call(
        &self, ctx: &LlmRunContext, pin: &ProviderPin
    ) -> Result<(), JobError>;

    // SQL append + commit + publish. Safe metadata-only.
    async fn record_llm_activity(
        &self, ctx: &LlmRunContext, activity: LlmActivity
    ) -> Result<(), JobError>;

    // DB lookups, pin verification and settings/authorization queries
    // formerly spread across services/background/models.rs / providers.rs.
    async fn resolve_pinned_llm_provider(
        &self, config: &AppConfig, pin: &ProviderPin
    ) -> Result<ResolvedLlmProvider, JobError>;

    async fn authorize_background_model_call(
        &self, config: &AppConfig, job_id: &str, pin: &ProviderPin
    ) -> Result<(), JobError>;
}
```

Return typed snapshots/identities without credentials in events. A runtime Provider snapshot may carry secret material **only in memory through existing credential resolution**, never into Job Event payloads or Admin responses. Keep a single actual implementation; do not introduce speculative trait adapters, a new SQL repository framework or a second DB pool.

### 5.2 Responsibility map / concrete refactor

| File | Permitted logic | SQL policy |
|---|---|---|
| `database/jobs/orchestration.rs` | Budget reservation, Job checkpoint, transactional start event | **SQL allowed / owned** |
| `database/jobs/events.rs` | Validate bounded event metadata, insert/read events, commit, broadcast after commit | **SQL allowed / owned** |
| **New** `database/jobs/providers.rs` | `providers` reads, Provider pin/canonical option resolution, background enabled and requester checks | **SQL allowed / owned** |
| `database/jobs/mod.rs` | Register DB Modules, expose the minimum typed Database methods | **SQL allowed** |
| `services/background/orchestration.rs` | Decide Planner/Repair/Replan/Worker, allocate Run ID, compile/validate proposal, invoke DB methods | **NO SQL/DB connection borrowing** |
| `services/background/models.rs` | Time provider acquisition, manage provider leases and capacity, call typed DB pre-dispatch check, invoke model | **NO SQL/DB connection borrowing** |
| `services/background/engine.rs` | Submit/route Job and apply settings through typed DB methods | **NO SQL/DB connection borrowing** |
| `services/background/providers.rs` | Remove after its DB-backed `pin/options` are moved, or retain only pure ref parsing | **NO SQL** |
| `providers/llm/openai/responses.rs` and other LLM adapters | Parse/model HTTP, return typed completion + usage; no Job persistence | **NO SQL** |
| `app/admin/background.rs` | Authenticate/validate HTTP and map DTOs; call engine/db methods | **NO SQL** |

**Required move from existing main:** 
- `models.rs::acquire()`: replace `database.pool().acquire()`/raw provider SELECT/`providers::pin` with `Database::resolve_pinned_llm_provider(...)`; call existing `ProviderRuntimeManager` with the returned exact snapshot.
- `models.rs::complete_inner()`: replace raw `SELECT enabled FROM background_settings`, `authorize(...)` and exact re-pin query with `Database::authorize_background_model_call(...)`. Respect existing publication guard and cancellation/revocation order; keep backend admission semantics.
- `engine.rs`: replace direct DB connection use in settings update, `validate_pair()` and provider options with typed `Database` methods.
- `services/background/providers.rs`: migrate config/database pin construction and Provider options to `database/jobs/providers.rs`; update all references, including `database/jobs/orchestration.rs`'s current cross-call back into `services/background/providers::pin`, avoiding the old circular SQL ownership.
- For all changed files, verify no `sqlx::query*`/`pool().acquire()` remains outside `database/`. Do not expand this task into unrelated preexisting features.

### 5.3 Provider lifetime, usage and provider-specific transport

At each call: `reserve_model_call` (run started) -> exact pinned Provider resolution -> runtime lease -> capacity permit -> exact pre-dispatch DB authorization -> `llm_request_started` -> `background_complete` -> typed usage / output -> Rust validation -> `llm_run_finished`.

Actual `adapter/model/transport` are recorded only after exact Provider resolution. The "request dispatched" UI stage corresponds to **the background completion invocation starting**, not to an acknowledged upstream packet. Persist `provider_wait_ms` at acquisition and `capacity_wait_ms` at call start/failure. On provider failure before acquisition: only pinned identity + terminal failure.

Change only the **Background** LLM completion result where necessary, e.g. `BackgroundCompletion { text: String, usage: Option<TokenUsage> }`; retain existing `JobModels` testing seam and avoid inventing a separate observer trait. `TokenUsage` may contain input/output/reasoning/cached counts; absent upstream fields stay absent. Responses uses actual `usage`; Chat-completions adapters may legitimately return `None`. Preserve `LlmProvider::stream` and Voice `/chat/completions`; Responses opted-in routing and no silent fallback remain per ADR 0099.

**Termination guarantee:** ensure once a Run is reserved, all normal success/error/timeout/cancellation return paths attempt to emit `llm_run_finished`. No new retry is permitted to fill missing telemetry; restart-interrupted runs are derived as unconfirmed from existing Job terminalization.

## 6. Frontend structure (PONYTAIL without changing View A)

Implement a small number of production Modules while reproducing all approved visible panels:

| Module/file | Work |
|---|---|
| `pages/jobs/JobDetailPage.vue` | Keep route, `DetailHeader`, actions, SSE load/cursor, and render 4 KPI cards + approved View A grid; preserve existing Step/Artifact/Result UI below. |
| **New** `components/jobs/JobLlmTimeline.vue` | Timeline rail, cards, inline expandable stages, **persistent right Selected LLM Run and Execution Progress**, selection state. One Module, not three thin Vue wrappers. |
| **New** `domain/jobs/llm-activity.ts` (or adjacent pure helper) | Project existing `JobEvent[]` into Run cards, metrics, progress; deterministic/read-only, unit-testable. |
| `api/types/agent-jobs.ts` | Extend event type and safe payload types, leave existing `JobEvent` envelope unchanged. |
| Existing VI/EN i18n catalogs | All labels, statuses, metric subtitles, no hard-coded English outside existing translation practices. |
| `api/agent-jobs.ts`, `api/job-updates.ts` | **Reuse** REST/SSE; no new polling or stream. |

A separate `JobLlmRunCard.vue` or `JobLlmRunInspector.vue` is **not required for v1**: keep sub-rendering inside `JobLlmTimeline.vue` until there is a demonstrated second caller/complexity. **The persistent Inspector panel itself is required**, even if not a separate Vue file.

### Realtime, paging and projection

- Use existing `GET /api/admin/jobs/{id}` + cursor-based `GET /api/admin/jobs/{id}/events` and `GET /api/admin/jobs/stream`. `JobChangeFeed` publishes after commit; SSE carries only Job ID; REST remains authoritative.
- On SSE ready/change/resync, visibility and fallback: reuse the current `JobDetailPage.vue` reconcile loop; do not add a separate stream just for LLM Runs. The current `main` loads event pages until a page shorter than 100 (with a route-aware abort).
- Group events by opaque `run_id`, sort by **first sequence ascending** as in approved View A, ignore malformed/unknown future event types safely, and preserve legacy events in their secondary view.
- Preserve selection by `run_id` across event refresh; for one selected card, clicking again toggles stages. If a Run disappears because the Job route changes, choose the first available Run. Do not auto-scroll or auto-select newest activity.
- If a Job has very many events, performance tuning may be a later feature. Do not introduce a parallel "50 cards" pagination state or extra Load More controller in v1 without demonstrated need; existing authoritative cursor loading already solves correctness for normal bounded Jobs.
- Refresh the four KPIs and Execution Progress from actual events/Job read model. Display running elapsed through a lightweight local display timer only where needed; no fake backend activity.
- One-to-one UI source-of-truth: no second full LLM activity rendering inside the existing raw Event Timeline by default.

## 7. End-to-end example: invalid Plan -> Repair -> Worker

```text
09:42:01  Run 01 / Planner       llm_run_started      pinned openai-primary
          Provider Acquired    llm_provider_acquired  OpenAI / gpt-5 / Responses
          Request Dispatched  llm_request_started   capacity wait: 48ms
          Response Received   llm_response_received observed usage available
          Validation Rejected plan_rejected         unknown_operation at $.steps[1].operation
          Run Failed          llm_run_finished      safe validation_code/path
09:42:06  Run 02 / Plan Repair   llm_run_started      same pinned provider, attempt 2
          Provider Acquired    llm_provider_acquired  OpenAI / gpt-5 / Responses
          Request Dispatched  llm_request_started
          Response Received   llm_response_received
          Run Completed       llm_run_finished      validation passed
          Plan Accepted       plan_accepted         Rust PlanCompiler checkpoint
09:42:09  Run 03 / Worker        llm_run_started      worker pin; step=synthesize_result
          Provider Acquired    llm_provider_acquired  model from exact runtime snapshot
          Request Dispatched  llm_request_started
          Waiting Response... no fabricated token stream
          Run Completed       llm_run_finished      after JSON/schema validation
          Step Finished       step_finished         Rust-owned checkpoint
          Job Finished        job_finished          terminal Job state
```

Both the rejected Planner Run and successful Repair stay visible. Right Inspector displays exact identity/usage for whichever card Admin selects. Rejection is **not** equivalent to Job failure; Worker response success is **not** equivalent to Step success until Rust validates/checkpoints it.

## 8. Delivery phases

**Phase 1 — Database-first**
- Add forward migration expanding six existing `job_events` types by five LLM types (keep old rows/FK/sequence/payload cap).
- Move touched Background Provider/Settings/authorization SQL into `database/jobs/providers.rs`; expose typed Database Interfaces and eliminate direct SQL/connection acquisition from the touched `services/background` logic files.
- Extend atomic `reserve_model_call` and typed `record_llm_activity`; test quota rollback, event order, no raw secrets, and after-commit SSE hint.

**Phase 2 — LLM instrumentation**
- Allocate `run_id` per Planner, Repair, Replan and Worker attempt; pass context through existing `JobModels` seam.
- Observe Provider acquisition/capacity/dispatch/result; typed Responses usage; preserve exact pin, timeouts, cancellation and quarantine.
- Validate Plan errors and attach safe `run_id` diagnostics without modifying Rust Plan compiler authority.

**Phase 3 — Approved View A**
- Implement 4 KPI cards, chronological rail, expandable Run cards, selected Run right panel and Execution Progress panel **to match the approved HTML**.
- Preserve existing Step/Artifact/Result features below, Job actions, styling tokens, responsiveness and VI/EN translations.
- Reuse existing REST/SSE/Job Detail state. No filters, modal Inspector, alternative layout or fake progress.

**Phase 4 — Qualification**
- Test failed Planner -> Repair -> Worker; provider mismatch/unavailable; capacity timeout; cancellation; restart; old Jobs; event pagination/reconnect; token-usage absence and retention.
- Confirm no new SQL/raw SQLite connection in logic Modules, no exposure of keys/raw prompts, correct display identity after Provider settings change, and no changes to Voice transport.
- Run project checks through `scripts/agent-test.sh` (not direct `cargo test`); targeted Rust tests first, then web test/typecheck/build and relevant full checks.

## 9. Acceptance criteria

- [ ] **Approved View A visual structure**: real Studio shell + Job header + **4 KPIs + left timeline + persistent right Selected LLM Run + right Execution Progress**. Selecting a card updates Inspector and expands stages; clicking again toggles expansion. No filter bar or modal.
- [ ] Original Steps/Artifacts/Result/Job actions still work, placed below the approved primary View A, without replacing its right-hand panels.
- [ ] Durable `llm_run_started` shares model budget transaction; a failed provider still yields a Run with pinned Provider displayed.
- [ ] Each Planner/Repair/Replan/Worker invocation correlates by unique `run_id`; provider/model/transport reflect the **resolved pinned runtime**, not mutable current config.
- [ ] Five (not seven) additive LLM events are enough for provider activity, response, safe validation diagnostics and Run terminal status; existing six Job events remain available.
- [ ] The UI does not claim upstream request ACK, token streaming, usage where unavailable, Plan acceptance before Rust compile, or terminal success without checkpoint.
- [ ] **All SQL/SQLx queries and DB pool/connection acquisition in the touched Agent Job flow are located under `crates/voice-agent-server/src/database/`**; `services/background/*` and Admin handlers call typed Database methods. Migrations remain under `migrations/`.
- [ ] No unauthorized change to pin checks, budget, cancellation, quarantine, recovery, plugin authority, Voice LLM, or existing retention.
- [ ] SSE reconciliation reuses existing code; Job events are ordered by `sequence` and support previous Jobs.
- [ ] Bilingual, responsive, keyboard accessible, light/dark theme; no prototype switcher or mock data shipped.
- [ ] Tests/checks run using `scripts/agent-test.sh`; document any inability to execute tests instead of claiming success.

## 10. PONYTAIL decisions recorded

| Review recommendation | v1.1 decision | Reason |
|---|---|---|
| Reduce 7 lifecycle types to 5 | **Accept** | Five events + existing plan/step events show the approved timeline. |
| Remove thin `JobLlmActivity` wrapper | **Accept** | Existing Database/`events.rs` owns durable writes. |
| Shrink Vue component proliferation | **Accept internally** | One timeline Module + pure projection, but all View A visible panels remain. |
| Replace Inspector with inline only / modal | **Reject** | Approved HTML requires a permanent right **Selected LLM Run** panel plus inline stages. |
| Delete 4 KPI cards | **Reject** | Approved HTML includes them. |
| Add All/Errors-only filter | **Reject** | Approved View A has no filter bar. |
| Two role values; derive Plan Repair/Replan | **Accept** | Explicit phase/attempt ensures distinct labels without four role kinds. |
| Put provider-call start in managed runtime | **Accept** | Accurate bounded event after admission; UI text is shorthand, not network acknowledgment. |
| Do not add recovery queries | **Accept** | Existing `job_finished` is enough for derived interrupted state. |
| Reuse cursor data load | **Accept** | Main already supports paged reads, no second pagination subsystem. |
| Shrink duplicate examples | **Accept** | One flow plus acceptance checklist. |
| **User requirement: strict SQL locality** | **Mandatory** | Provider/Settings SQL currently exists in logic; relocate into `database/`. |
| **User requirement: exact approved UI** | **Mandatory** | View A HTML is stronger than generic simplification suggestions. |

**Implementation status:** Code was added to the feature branch. The implementation record below distinguishes committed work from verified tests.

## 11. Implementation record (2026-10-14)

The branch now includes implementation commits for:
- New forward migration `0111_job_llm_activity.sql` and five additional bounded `job_events` types.
- Atomic model budget reservation + `llm_run_started`, durable Run finish/error and provider/capacity/request/response metadata.
- Optional typed Responses usage for OpenAI/OpenCode, without changing Voice streaming or background fallback policy.
- All SQL in the **touched** Agent Job admission/runtime/scheduler/step-result paths moved under `database/jobs/*`; removed the old `services/background/providers.rs`. Existing unrelated Plugin/Artifact SQL refactors are out of scope for this change.
- Exact View A structure in Admin Web: four KPIs, Run cards + stages, permanent right Inspector/Execution Progress, existing Job sections below, and authenticated SSE reconciliation.
- EN/VI labels, pure Run projection, unit tests, a Rust provider credential reload regression from the concurrent `main` fix, GLOSSARY and ADR 0102.

**Verification:** Per-file GitHub commit/content reads and a static 14-file contract scan passed, including SQL locality in the four touched Background logic files, parity of 44 EN/VI LLM labels, presence of all five event types and View A UI/SSE wiring. This does **not** replace Rust compilation, SQLx migration qualification, `vue-tsc` or real integration tests. The current execution environment has no Rust toolchain or complete local checkout, so `scripts/agent-test.sh` was not executed. Run the checks in Section 9 before merging.

**Concurrent `main` change:** `3e76bd3` adds canonical Provider credential references after idle eviction. That behavior and its regression test were explicitly incorporated into the feature branch's new Database Provider Interface. The branch may remain behind `main` by that commit in Git history until rebased or merged.
