# Agent Job — LLM Activity / View A (Timeline First)

> **Status:** Design v1.0 — ready for implementation review.  
> **Baseline:** `main` (reviewed 2026-10-14). **Working branch:** `feature/job-llm-activity-view-a`.  
> **Scope:** Admin Web Job Detail + Rust Background Job observability. No production code changes in this design commit.  
> **Decision:** View A — Timeline First, with on-demand Run Inspector (not permanent three-column split).

## 1. Outcome and scope

When an Admin opens `Jobs > Job Detail`, the primary activity display must show **a durable `LLM Run Started` entry as soon as a model invocation is reserved**, even while the provider is unavailable or waiting for capacity. The first card identifies the pinned Orchestrator/Worker Provider; subsequent entries explain **which provider/model/transport really executed**, wait times, request/response boundaries, validation, Plan Repair, Worker outcomes, and Job completion.

A model **Run** (also called *LLM Invocation* in implementation discussion) is **one bounded call to a model**, not an Agent account, Voice Session, Job, Job Step, or OpenAI Responses background task. Each Planner/Repair/Replan/Worker model call receives a unique `run_id`. Rust remains the authority for Plan compilation, Job status, Step checkpoint, security and recovery.

### Goals

1. Start the visible LLM log at admission of each actual model invocation: `llm_run_started` is committed atomically with the existing model-call budget reservation, **before provider acquisition**.
2. Identify provider reference, source, pinned revision/incarnation, and later the **resolved adapter, model and actual transport** (`responses` or `chat_completions`) without looking up mutable current settings for historical records.
3. Show lifecycle, validated outcomes, bounded diagnostics and real usage metadata as events appear; work for both failed and successful runs.
4. Maintain the current Job Detail header, sidebar/navigation, action buttons, Step/Artifact/Result data, i18n and palette; preserve desktop/mobile behavior.
5. Reuse SQLite `job_events`, Admin bearer authorization, `JobChangeFeed` and `GET /api/admin/jobs/stream`; REST is the authoritative source and SSE is only a hint.
6. Keep preexisting Jobs, `job_events` and clients readable; do not modify scheduler execution semantics or retry behavior.

### Out of scope

Raw prompts, complete model output, hidden chain-of-thought, token-stream display, live plugin stdout, tracing infrastructure, OpenTelemetry, Redis, new WebSocket channel, a new top-level Admin page, configuration toggles per Agent, or changing `Voice Session` routing. The `Responses` adapter remains one-shot `store:false`, `background:false`, `stream:false` per ADR 0099; **no simulated token progress**.

## 2. Verified main baseline (2026-10-14)

| Concern | Existing seam/file | Observation |
|---|---|---|
| UI | `apps/admin-web/src/pages/jobs/JobDetailPage.vue` | Compact `DetailHeader`; 2-column `xl:grid-cols-[minmax(0,1.45fr)_minmax(280px,1fr)]`; left Steps/Artifacts/Result, right Execution Context/Event Timeline; Cancel/Retry/Refresh. |
| Styling | `apps/admin-web/src/style.css` | Semantic Tailwind tokens, `studio-panel`, violet/cyan accents, light/dark themes. |
| UI contracts | `apps/admin-web/src/api/types/agent-jobs.ts` | `AdminJob`, six `JobEventType` values; paged events `{items}`. |
| UI data | `apps/admin-web/src/api/agent-jobs.ts` | `job()`, `events(after, limit=100)`; no dedicated LLM API. |
| Realtime | `apps/admin-web/src/api/job-updates.ts` | Authenticated fetch SSE, resync on lag/reconnect and aborted streams; 60-second fallback in Job Detail. |
| Admin routes | `crates/voice-agent-server/src/app/admin/mod.rs` | `/jobs/{id}`, `/jobs/{id}/events`, `/jobs/stream` are behind Admin auth. |
| Persistence | `crates/voice-agent-server/src/database/jobs/events.rs`; migration `0107_job_execution_events.sql` | Append-only `job_events` with per-Job sequence and payload <=4096 bytes; CHECK restricts types to six existing events. |
| Model reservation | `database/jobs/orchestration.rs::reserve_model_call()` | Updates planner/step budget; Planner appends `plan_attempt` in same transaction. |
| Orchestration | `services/background/orchestration.rs` | `propose()` handles initial/repair/replan, Worker calls `complete_structured()`, Rust validates output and checkpoints Step. |
| Runtime | `services/background/models.rs` | Verifies pinned provider, acquires lease/capacity, authorizes again and calls `runtime.background_complete()`; wait times currently mostly only in `tracing`. |
| Provider pin | `services/background/model.rs::ProviderPin` | `reference`, `incarnation`, `revision` and `digest`; model/adapter are **not** present in `JobSnapshot`. |
| Responses | `providers/llm/openai/responses.rs` | Extracts usage metadata for `tracing` but background result is currently only `String`. |
| Job recovery/retention | `database/jobs/recovery.rs`, `services/background/artifacts.rs` | Restart terminalizes interrupted Jobs; retention deletes `job_events` before parent Job. |

**Compatibility decisions:** ADR 0098 keeps untrusted Plan Proposal and Plan Repair metadata-only; ADR 0099 preserves Voice Chat Completions and opts Agent Jobs into Responses; ADR 0100 makes SQLite authoritative with best-effort SSE. This document extends those decisions, rather than replacing them.

## 3. View A — screen composition

### 3.1 Desktop layout (>=1280px)

Keep existing page route, Admin sidebar, compact `DetailHeader`, and the two-column grid. Move the new **LLM Activity** section to the **top of the wide left column**. Place Steps, Artifacts, and Result below it (collapsible only where already supported). Keep **Execution Context** and the **existing Job Event Timeline** in the narrower right column. Avoid a permanent third column.

```text
Admin sidebar   Jobs / Job Detail
┌──────────────────────────────────────────────────────────────────────┐
│ ← Jobs  Job ID / Goal         [Running] [Refresh] [Cancel / Retry] │
├──────────────────────────────────────────────────────────────────────┤
│ [LLM Calls 04] [Errors 01] [Known Tokens 1,820] [Elapsed 00:23]    │
├──────────────────────────────────────────┬───────────────────────────┤
│ LLM Activity                  ● Live     │ Execution Context         │
│ [All] [Planner] [Worker] [Errors]        │ Job status / requester    │
│                                          │ Plan rev / Budget         │
│ ● LLM Run #1  Planner        Failed      │                           │
│   Provider: openai-primary • Responses   │ Existing Job Timeline     │
│   Model: ... • Attempt 1 • 3.8s          │ plan_attempt / rejected   │
│   ▾ Run stages / validation              │ plan_accepted / step_*    │
│                                          │                           │
│ ● LLM Run #2  Repair         Completed   │                           │
│   Provider: openai-primary • Responses   │                           │
│   Model: ... • Attempt 2                 │                           │
│                                          │                           │
│ ◉ LLM Run #3  Worker         Running     │                           │
│   Provider: worker-llm • Chat            │                           │
│   [Inspect run]                          │                           │
│                                          │                           │
│ Job Steps (existing)                     │                           │
│ Artifacts / Result (existing)            │                           │
└──────────────────────────────────────────┴───────────────────────────┘
```

This wireframe is illustrative; labels/models must come from the persisted run data. Prefer card density comparable to `studio-panel p-5`, `text-xs` metadata and current Lucide icon family. No oversized hero/banner or floating Debug console.

### 3.2 Tablet/mobile (<1280px)

- Existing columns stack into one; order: Header -> Job warning -> compact KPIs -> LLM Activity -> Steps -> Artifacts/Result -> Execution Context -> existing Job Timeline.
- Filters scroll horizontally if needed; all essential identifiers wrap/truncate safely; run card is full-width.
- Click a card's **Inspect**/chevron to open the **Run Inspector** as a `Dialog`/drawer on desktop; on mobile use an accessible full-width modal or inline expandable details, **not** a fixed multi-column inspector.
- Preserve Job's current Cancel/Retry/Refresh behavior and confirmation flows. No new sidebar item.

### 3.3 Per-Run card: collapsed vs expanded

**Collapsed:** status icon + "LLM Run #N", role label (`Planner`, `Plan Repair`, `Replan`, `Worker`), attempt, pinned provider display name/ref, model if resolved, actual transport if known, elapsed/duration and linked `step_key`. Prioritize error code on failed cards, not the full JSON diagnostic.

**Expanded/Inspector:** ordered lifecycle entries with time and durations, provider identity, request metadata, response metadata, plan validation diagnostic (`code`, `path`, short safe `message`), related Plan revision/Step, optional usage. Show an explicit **"Not available"** for usage/model/transport not yet resolved; never infer values from currently selected Provider settings. A Run can be `waiting_provider`, `waiting_capacity`, `requesting`, `validating`, `completed`, `failed`, `cancelled` or **`interrupted` (derived, see section 6.6)**.

The **Job status** and **Run status** remain separate. "LLM response received" does *not* mean "Plan accepted". Only a committed `plan_accepted` confirms publication; only `step_finished`/`job_finished` confirm domain checkpoints.

### 3.4 View states

| Job / data state | UI behavior |
|---|---|
| `queued` | Show "Waiting for scheduler"; zero LLM Runs; do not fabricate a start event. |
| `planning` but no Run yet | Show "Preparing first model invocation"; normal loading state. |
| Run reserved but provider unresolved | Show pinned `provider_ref` and `source`; `Model: resolving`; status "Acquiring provider". |
| Provider pin mismatch/unavailable | Keep the failed Run visible with `configuration_changed` / `llm_provider_unavailable`, no dispatch checkmark. |
| Capacity blocked | "Waiting for capacity" and elapsed time from committed start; no fake progress %. |
| Responses in flight | Spinner/"Waiting for model response"; **no streaming tokens** because background Responses is one-shot. |
| Validation rejected | Failed validation diagnostic; show next Repair Run separately if started. |
| Step/Job terminal | Completed/failed/cancelled display; in-progress Run with no terminal event is "interrupted/unconfirmed" (not success). |
| Lost SSE/reconnect | REST reconciliation via existing event cursor, visibility trigger and fallback; preserve selected Run. |
| Old Job without new events | Show existing Job Timeline/Steps normally and "Detailed LLM tracing wasn't available for this Job". |
| Empty/fetched partial/error | Stable skeleton/empty and retry affordance; do not silently show a complete timeline if paging truncated. |

### 3.5 Interaction and accessibility

- Default filter `All`, plus `Planner`, `Worker` and `Errors`. `All` includes repair and replan; Planner includes planner/repair/replan. Filters are client-side over loaded events, not server mutation.
- When a Job is running, auto-expand **the newest active Run** only when the Admin has not selected another; on failure, offer a prominent inspection affordance. Do **not** auto-scroll while the user reads older logs.
- Each Run card has one click/keyboard target (`button` with `aria-expanded` / accessible label); event row is not a nested link. Copy Run ID and error code with existing clipboard UI primitives, not raw request/response.
- Timestamps use the existing locale/i18n functions; durations use measured milliseconds. Never imply millisecond precision for `job_events.created_at`, which is stored in Unix **seconds** today. Ordering comes from per-Job sequence.
- Preserve theme tokens from `style.css`, `Badge` and `Button` patterns, native keyboard focus, reduced-motion, error contrast and English/Vietnamese dictionary entries.
- Provider, model, IDs and code paths are text only; render through normal Vue interpolation (no `v-html`).

## 4. Durable event contract

### 4.1 Extend `job_events`, do not create a new log database

Add a **new migration** (next available sequential migration number on implementation branch) that recreates `job_events` with the existing data and constraints plus the new permitted event types below. Do **not** edit migration `0107` or relax the 4096-byte cap. Retain `PRIMARY KEY(job_id,sequence)` and the Job FK. Backfill is **not** required for old Jobs; existing six event types remain valid.

**New append-only types (v1):**

| Event type | Stage | Mandatory bounded payload | Optional safe payload |
|---|---|---|---|
| `llm_run_started` | Budget reserved, **before acquisition** | `run_id`, `role`, `attempt_no`, `provider_ref`, `provider_source`, `pin_revision` | `step_key`, `plan_revision`, `pin_incarnation` |
| `llm_provider_acquired` | Exact pinned provider resolved and lease acquired | `run_id`, `adapter`, `model`, `transport` | `provider_wait_ms` |
| `llm_capacity_acquired` | Admission permit obtained | `run_id`, `capacity_wait_ms` | — |
| `llm_request_started` | Background provider transport about to issue the request | `run_id`, `transport` | `message_count`, `input_bytes` |
| `llm_response_received` | Provider returned a completed text response | `run_id`, `output_bytes` | `request_duration_ms`, `input_tokens`, `output_tokens`, `reasoning_tokens`, `cached_input_tokens` |
| `llm_validation_finished` | Rust checked output | `run_id`, `status` (accepted/rejected) | `validation_code`, `validation_path`, `validation_message` |
| `llm_run_finished` | Terminal invocation outcome | `run_id`, `status` (completed/failed/cancelled) | `error_code`, `duration_ms`, `outcome_known` |

**Existing types remain** `plan_attempt`, `plan_rejected`, `plan_accepted`, `step_started`, `step_finished`, `job_finished`. Add `run_id` only to the payloads for directly related plan attempts/rejections when context is available; do not reinterpret existing Plan/Step ownership or rewrite history.

The persisted `role` enum is `planner` | `plan_repair` | `replan` | `worker`. A repair of an initial Plan uses `plan_repair`; a repair of an append-only Replan also uses `plan_repair` with `plan_revision`/`phase` identifying the replan context. `attempt_no` is 1-based **within the corresponding proposal round**, not necessarily the lifetime budget counter; `run_id` is globally unique (UUID).

**No event is emitted for every model token or plugin stdout line.** Normal budget-constrained calls yield a maximum of seven LLM-specific events plus existing Job/Plan/Step checkpoints. Unknown/absent fields are omitted rather than fabricated as zero.

### 4.2 Example REST envelope

Reuse `GET /api/admin/jobs/{id}/events?after=SEQ&limit=100`. `JobEvent` shape stays `{sequence,event_type,created_at,step_key,payload}`. A new event example:

```json
{
  "sequence": 8,
  "event_type": "llm_run_started",
  "created_at": "2026-10-14T02:42:01Z",
  "step_key": null,
  "payload": {
    "run_id": "eeb86c41-1c1f-4c6e-b617-7f20b908f976",
    "role": "planner",
    "attempt_no": 1,
    "provider_ref": "config:llm:openai-primary",
    "provider_source": "config",
    "pin_revision": 4,
    "pin_incarnation": 0
  }
}
```

**Provider identity:** `JobSnapshot` freezes the source-qualified Provider pin but not adapter/model. At `llm_run_started`, always persist the **expected pinned ref and revision**. After the **same** pin is verified/resolved, `llm_provider_acquired` stores the exact adapter/model/transport selected for that invocation. This is the first authoritative actual-provider identity; do not read mutable Admin Provider Catalog on historical page loads. In `configuration_changed`/unavailable cases, display the known pin and mark actual model/transport `unresolved`.

`llm_request_started` means entry into the **background provider request adapter** (dispatch initiation), not proof that bytes reached the remote endpoint. An orchestration-layer call into `ManagedModels::complete_inner` is **not** sufficient: instrument at the actual adapter entry. UI label: **Request started**, never **Request delivered**. Preserve this event name across the migration, Rust and frontend. Do not log `llm_response_received` for transport errors.

### 4.3 Run derivation, ordering and budgets

- `reserve_model_call()` is the only budget admission path. Extend it to accept typed `LlmRunContext`, and **atomically** update `budget_used_json`, append `llm_run_started`, and (for Planner) append the existing `plan_attempt`. Commit before invoking Provider. If reservation fails, no Run exists; show Job's real error instead.
- All events append with the current `job_events` sequence, under SQLite short transactions; publish `JobChangeFeed` **after commit**. Never hold a SQLite write transaction across provider await/capacity wait.
- Keep monotonically ordered events by `sequence`, not wall-clock timestamp. `run_id` is attached to every LLM-specific event and relevant Plan diagnostic.
- The provider is resolved against the exact `ProviderPin` and checked again before use, as today. No new permission or automatic provider fallback.
- Add a bounded pure frontend projection `projectLlmRuns(events, job.status)` that groups by `payload.run_id`, sorts by first `sequence` and derives UI stage/status; late/missing/duplicate event reads must not crash it. Old events remain in the legacy timeline only.
- Do not extend `JobSnapshot` or expose provider secrets to achieve historical display.

### 4.4 Durability, migration and retention

- Use a new SQLite SQLx migration to rebuild/check the permitted `event_type` set. Copy old rows, retain exact old sequences and referential constraints. Treat a partially applied migration as an error; verify upgrade on populated DB and on empty DB. Do not disable foreign-key integrity as an ongoing server setting.
- `job_events` stays metadata-only, payload <=4096 bytes (Rust validates before insert, SQLite enforces). Bound field lengths: for example model/ref <=128 bytes, `validation_path` <=256 bytes, diagnostic message <=256 bytes; use stable diagnostic codes instead of raw provider response bodies.
- Existing terminal retention in `services/background/artifacts.rs::expire_terminal` deletes `job_events` before deleting `jobs`, so LLM activity follows the Job's existing retention; no separate GC.
- Provider/Worker outcomes remain the source of execution truth; a lost activity event must not synthesize a success. Any **terminal checkpoint** and its related domain events should commit atomically where practical. Non-critical progress append failure should be recorded in server `tracing` and must never trigger a second remote LLM request.
- A missing `llm_run_finished` after restart is **not** evidence of completion. For display, if Job is terminal, project a derived `interrupted` label with "no terminal Run checkpoint"; optionally emit a recovery closure event only as a later explicitly tested enhancement. Retain the existing unknown-outcome/quarantine semantics and no automatic replay.

## 5. Rust Module design (deep interface)

### 5.1 Placement

Create `services/background/llm_activity.rs` for **`JobLlmActivity`**, a deep Module hiding event formats, safety limits, identity and lifecycle mapping. Use the existing `database/jobs/events.rs` as the durable append seam (with narrow typed helpers if needed); keep provider transport independent of Admin UI DTOs.

```rust
pub(crate) struct LlmRunContext {
    pub run_id: uuid::Uuid,
    pub job_id: String,
    pub step_key: Option<String>,
    pub role: LlmRunRole,
    pub attempt_no: u32,
}

impl JobLlmActivity {
    // Reservation is atomic with the existing model-call budget update.
    pub(crate) async fn begin(
        &self, ctx: &LlmRunContext, pin: &ProviderPin,
    ) -> Result<(), JobError>;

    // Bounded metadata-only activity; publishes only after SQLite commit.
    pub(crate) async fn record(
        &self, ctx: &LlmRunContext, activity: LlmActivity,
    ) -> Result<(), JobError>;
}
```

This is an **illustrative Interface**, not a code signature to paste unmodified. Prefer plumbing `ctx` through the existing `JobModels` seam over a task-local/global singleton, adding an optional span or callbacks only if required by the actual provider code. Do **not** introduce a second persistent `job_llm_calls` table in v1: it would duplicate authoritative event history and complicate cleanup/consistency.

### 5.2 Call sites and ownership

| Existing file | Required change |
|---|---|
| `services/background/orchestration.rs` | For each call in `propose()` and Worker Step, allocate unique `run_id` and typed role/attempt; invoke atomic reservation/start; emit typed Rust validation outcome; relate rejections to `run_id`; preserve Repair/Replan controls and limits. |
| `database/jobs/orchestration.rs` | Extend `reserve_model_call` so quota increase and `llm_run_started` are one `BEGIN IMMEDIATE` transaction; keep original `plan_attempt` and commit order. Optional `run_id` in `reject_plan` payload, no added Job status. |
| `services/background/models.rs` | Pass `LlmRunContext` through managed model call, record provider acquired/capacity wait/request/response lifecycle; preserve existing cancellation, deadline, exact pin check, admission and quarantine. |
| `services/background/providers.rs` | Reuse existing pin resolution; surface only safe resolved provider identity from the validated snapshot; no new mutable display lookup. |
| `providers/llm/mod.rs` and `providers/llm/openai/responses.rs` | Background-only typed completion metadata for actual model/transport/usage, e.g. `BackgroundCompletion { text, usage: Option<Usage> }`. Chat-only fallback may return `usage: None`. Keep `LlmProvider::stream()` unchanged for Voice. |
| `providers/llm/opencode_free.rs` | Report actual background transport selected by its explicit capability (no fallback), not by guessed model name. |
| `database/jobs/events.rs` | Bounded append, page existing events, extend safe types and ensure post-commit invalidation through existing feed. |
| `database/jobs/recovery.rs` | Qualify interrupted Run projections against Job terminal state, without automatic provider retry. |
| SQLx migration | Rebuild `job_events` CHECK while retaining data/FK/sequence constraints. |
| Existing Rust tests | Update `JobModels` fakes/qualification tests if signature changes; keep all prior plan/step/recovery behavior. |

**Metrics:** Prefer `BackgroundCompletion { text, usage: Option<...> }` over parsing `tracing` output. With Responses parse the existing `usage` payload; for Chat Comple­tions only record fields actually exposed by the provider/adapter. `reasoning_tokens` is a count, **not** hidden reasoning content. Missing usage remains `null`, not `0`. Model latency uses monotonic `Instant` around the actual awaited operation; wall-clock event timestamps remain durable seconds.

## 6. Admin frontend design

### 6.1 Modules/changes

| Existing/new file | Change |
|---|---|
| `pages/jobs/JobDetailPage.vue` | Insert LLM KPIs and `JobLlmTimeline` as first section of left column; keep `DetailHeader`, warning, Actions, Steps, Artifacts, Result and right aside. Reuse existing event fetch/SSE lifecycle. |
| `components/jobs/JobLlmTimeline.vue` (new) | Timeline list, filters, empty/loading/truncated states; callbacks for selecting Run. |
| `components/jobs/JobLlmRunCard.vue` (new) | Compact current/failed/completed Run, provider/model/transport summary and keyboard accessible expand. |
| `components/jobs/JobLlmRunInspector.vue` (new) | On-demand dialog or inline details with lifecycle, safe diagnostics, usage and step links. Prefer existing dialog primitives, not a new overlay system. |
| `domain/jobs/llm-activity.ts` (new, pure) | `projectLlmRuns`, statuses, role grouping, safe event type guards, totals; no HTTP or Vue state. |
| `api/types/agent-jobs.ts` | Widen `JobEventType` union, add typed optional metadata fields/projection types, keep old `JobEvent` shape unchanged. |
| `i18n/messages` catalogs | Add VI/EN strings for labels/empty/errors/filters/status; avoid raw English fallback in Vietnamese UI. |
| `api/agent-jobs.ts`, `api/job-updates.ts` | Keep existing REST and SSE endpoints, no extra socket or polling loop. |

### 6.2 Loading, cursor and reconnection correctness

Current UI loads an initial Job snapshot then pages Job Events by `after`, fetching up to 32 pages, and SSE requests a reconcile. Keep this pattern but **do not claim the timeline is complete when the cap is reached**. The projection must handle paginated data; add an explicit "Load older/more" or scheduled continuation until bounded convergence. Preserve dedupe by `sequence`.

The SSE `job_changed` payload contains only Job ID; Reactivity compares current route ID, schedules one coalesced refresh and REST loads after cursor. On `ready`/`resync_required`/visibility change, re-read. The 60-second fallback continues only when Job is active, as today. Handle route changes by cancelling previous requests; do not change the selected Run merely because one event was appended.

For high volume, render only the newest ~50 cards at once with a "Load more" affordance; older events can still be paged by existing REST. Cap DOM rendering independently of the durable event history. No unbounded frontend accumulation requirement for a long-running Job.

### 6.3 Status projection rules

- `llm_run_started` creates the card immediately with pinned ref, role and status "Acquiring provider". A second Run must never rewrite the first Run's provider.
- `llm_provider_acquired` fills adapter/model/transport; `llm_capacity_acquired` changes stage to "Ready to call".
- `llm_request_started` switches to "Requesting"; `llm_response_received` switches to "Validating" (for Planner/Worker).
- `llm_validation_finished` records validation success/failure; `llm_run_finished` closes Run. Plan acceptance remains linked to `plan_accepted`, Step success to `step_finished`.
- A Run with unknown future event names remains readable; partial/malformed payloads show placeholders and a safe warning rather than throwing.
- Running elapsed is a local display timer at most once per second **only for visible active cards**; never write fake events as a timer.

### 6.4 Security and UX constraints

- Admin bearer auth is already enforced by `app/admin/mod.rs`. Do not expose LLM activity over Device WS or public endpoints; no third-party analytics collection of event payloads.
- Never persist API keys, credentials, authorization headers, raw prompts, model response body, secret refs, plugin stdout or detailed source documents. A diagnostic code/path is allowed; message is a vetted bounded Rust string.
- Avoid provider detail hyperlinks for `config` Providers if they have no writable Admin detail route. Show plain text for unresolvable historical Provider rows. Never claim current Provider availability from a historical Run.
- Card content is user-visible **observation** and must not be used to drive Job state transitions.

## 7. End-to-end examples

### 7.1 Invalid proposal repaired

```text
[01] LLM Run Started     planner, pinned Orchestrator, attempt 1
[02] Provider Acquired   actual OpenAI/model/Responses
[03] Capacity Acquired
[04] Request Started
[05] Response Received
[06] Validation Rejected  unknown_operation, $.steps[1].operation
[07] plan_rejected        diagnostic; linked run_id
[08] Run Finished         failed_validation (UI projection from validation + terminal)
[09] LLM Run Started     plan_repair, same pinned Orchestrator, attempt 2
[10] Provider Acquired
[11] Request Started
[12] Response Received
[13] Validation Accepted
[14] Run Finished         completed (provider + validation)
[15] plan_accepted        executable Plan revision 1 committed
[16] LLM Run Started     worker, pinned Worker, step=synthesize
...
[NN] step_finished       checkpoint committed
[NN] job_finished        Job terminal
```

Note `failed_validation` is a **UI derived reason**, not necessarily a `llm_run_finished.status` enum. `llm_run_finished.status` remains `completed/failed/cancelled`, accompanied by a separate `validation_code`.

### 7.2 Provider unavailable / timeout / restart

- Unavailable: `llm_run_started` -> (no acquired) -> `llm_run_finished {status:"failed",error_code:"llm_provider_unavailable"}` -> Job fails under existing rules. UI must **not** display "request sent".
- Capacity timeout: start -> acquired -> (wait) -> finished with `deadline_exceeded`. No `request_started`.
- Responses refused/rejected: start -> acquired -> capacity -> request started -> failed with `llm_execution_failed`, no fake successful response event. Capture safe coarse error reason if qualified; keep old ambiguous outcome quarantine behavior.
- Restart: start (possibly acquired/request started) -> no terminal Run event -> existing `job_finished {error_code:"interrupted"}` from recovery. UI derives Run "interrupted (unconfirmed)" and makes **no replay**.
- Old Job: only `plan_*` and `step_*`. UI shows legacy timeline without inventing Run IDs or resolved model history.

## 8. Delivery phases (implementation order)

**Phase A — Durable contract and domain tests**
1. Write new migration to extend `job_events` CHECK; test upgrade retaining old records, FK integrity, ordering, retention.
2. Define typed LLM Run identity/metadata. Extend atomic `reserve_model_call`; assert quota failure creates neither budget increase nor start event.
3. Add lifecycle methods for bounded safe metadata and after-commit invalidation. Test failure, cancellation, restart and no raw outputs.

**Phase B — Managed model/provider instrumentation**
1. Thread `LlmRunContext` through Planner/Repair/Replan/Worker and `JobModels`; update fake implementations.
2. Capture provider resolution, actual transport, capacity wait, request boundary and timing without keeping SQLite transaction open across the await.
3. Add Background typed usage, Responses parsing and Chat fallback null usage. Verify Voice `stream()` transport never changes.

**Phase C — Admin UI View A**
1. Build pure projection + unit tests for fragmented event histories, two interleaved Run IDs, old events, incomplete Run and unknown new event types.
2. Build Timeline, cards and Run Inspector using existing components/tokens and VI/EN i18n.
3. Integrate with Job Detail layout/REST/SSE; verify live refresh, route changes, mobile keyboard behavior and no unnecessary network loop.

**Phase D — Qualification**
1. Run real Agent Job with Planner rejection -> Plan Repair -> accepted Plan -> Worker success and inspect identity/log ordering.
2. Exercise unavailable/config-changed provider, capacity timeout, cancellation, restart and event pagination/reconnect.
3. Verify large/long history, retention and existing Jobs. Check security by scanning actual event payloads (no prompt/response/auth secret).

## 9. Acceptance criteria

- [ ] New Run has a **durable start event** in the same transaction as budget reservation; the first UI card exists before provider acquisition even if provider fails.
- [ ] Card initially shows pinned provider; after exact validation it shows **actual adapter + model + transport**; historical views do not resolve from mutable current provider settings.
- [ ] Planner initial, Plan Repair, Replan and Worker calls have distinct IDs, linked attempts/Steps; no conflation with Plan/Job acceptance.
- [ ] All normal lifecycle stages are ordered by per-Job `sequence`, bounded, metadata-only and visible after commit through existing SSE+REST.
- [ ] Not-yet-dispatched, timed-out, cancelled, ambiguous and recovered cases never show fake `request_sent`/`completed` outcomes.
- [ ] Responses usage is captured as optional typed metadata; null when missing, without showing hidden reasoning text.
- [ ] Existing six event types, old Jobs, retention, Job Read View, voice transport, plugin execution and retry semantics remain unchanged.
- [ ] UI uses existing `DetailHeader`, `studio-panel`, two-column layout, existing actions and dark/light tokens. Responsive, keyboard accessible and bilingual.
- [ ] SSE resync / pagination does not lose or duplicate Run cards; >32 pages never appear falsely complete.
- [ ] Rust tests and web tests pass through repo-approved `scripts/agent-test.sh`.

Suggested checks, run **targeted before broad** (test names are implementation targets, not currently existing):

```bash
scripts/agent-test.sh targeted llm_run
scripts/agent-test.sh targeted plan_repair
scripts/agent-test.sh targeted job_events
scripts/agent-test.sh check voice-agent-server
scripts/agent-test.sh web test
scripts/agent-test.sh web typecheck
scripts/agent-test.sh web build
scripts/agent-test.sh package
scripts/agent-test.sh run -- cargo fmt --check
scripts/agent-test.sh full
```

## 10. Decisions and non-decisions

**Chosen:** one user-facing View A inside Job Detail; per-model-Invocation identity; additive durable events in existing `job_events`; optional on-demand Inspector; historical provider identity from the verified pin/runtime; existing authenticated SSE; metadata-only.

**Rejected for this iteration:** always-on three-column debugger, new top-level Logs page, second table `job_llm_calls`, unbounded event stream, raw request/response retention, provider fallback, queue-to-LLM fake Run, per-token progress, new broker, or speculative background model retry.

**Future optional design:** a separate opt-in, short-retention Diagnostic Capture Module may store sanitized request/response previews under explicit Admin access. It needs its own threat model and is **not authorized by this spec**.

**Implementation caveat:** This is an architecture/UI specification. No production code, migrations or tests were run as part of publishing this document. The event names, safe metadata typed shapes, and migrations are proposed contracts to implement and qualify on the new branch.
