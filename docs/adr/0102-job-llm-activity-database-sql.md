# ADR 0102 — Durable Job LLM Activity and Database-Owned SQL

Status: Accepted for implementation, 2026-10-14.

## Context

An Agent Job currently exposes durable Job/Plan/Step checkpoints but cannot explain which LLM Provider was acquired, which background transport was used, where capacity waits occurred, or why a particular Plan Repair was needed. These operations are neither a new Agent nor a Job Step. Existing provider queries also leak SQLx connection/query details into Background orchestration/runtime files, making the Provider-pin safety checks harder to maintain.

The accepted Admin UX is **View A: Timeline First** with four summary KPIs, a left chronological LLM Run timeline, and a persistent right Selected Run and Execution Progress inspector.

## Decision

One **LLM Run** is one bounded model invocation with immutable `run_id` and a source-qualified pinned Provider identity. `llm_run_started` is atomically written in the **existing model budget reservation** transaction, before Provider acquisition. Five bounded metadata-only events are appended to the existing SQLite `job_events` stream, identified by `run_id`: `llm_run_started`, `llm_provider_acquired`, `llm_request_started`, `llm_response_received` and `llm_run_finished`. Use existing `plan_attempt`, `plan_rejected`, `plan_accepted`, `step_finished`, and `job_finished` as Rust-owned domain checkpoints. Missing terminal activity after interrupted execution is **unconfirmed**, not success. Requests may enter a provider adapter without reaching the remote endpoint: UI describes the **provider call starting**, not upstream delivery/acknowledgement.

**All SQLx queries and connections for the touched Agent Job flow live in `crates/voice-agent-server/src/database/`.** Database Modules provide typed Provider resolution, pin checks, authorization, Job budget/event writes and Step reads; the Background orchestration/runtime and Admin route files use these small interfaces without embedded SQL. Existing security and exact Provider authority checks, cancellation deadlines, Voice reserved capacity, and ambiguous-outcome quarantine remain intact.

Background completion may return optional token-usage metadata; absent usage is unknown, not zero. Historical model and transport identity comes from the verified runtime/provider snapshot at the time of the invocation, **never** from current mutable Provider Catalog. Voice Chat Completions remains unchanged, and Background Responses remains one-shot with no remote state persistence, under ADR 0099.

SQLite remains the authoritative log read model. Existing best-effort `JobChangeFeed`/Admin SSE broadcasts after commit, and the Job Detail page refreshes `GET /api/admin/jobs/{id}/events` using its sequence cursor under ADR 0100. No new broker, log database, public endpoint, raw prompt/body retention or token stream is introduced.

## Alternatives considered

- A separate `job_llm_calls` table: rejected for this version; duplicates durable event lifecycle and requires extra reconciliation/retention.
- Writing log events from Provider adapters using direct database connections: rejected; couples shared Voice transport code to Job persistence.
- A new tracing Module with generic observer traits or a vendor background agent: rejected; duplicates one established database/runtime seam.
- A third-party tracing backend or WebSocket stream: rejected; the existing durable events and SSE already support the approved UI.
- Keeping SQL in `services/background/models.rs` because it needs provider admission: rejected; typed Database Interfaces preserve one owner for query/transaction details.

## Consequences

A forward SQLx migration adds five allowed event types without changing old rows. The existing event payload bound of 4096 bytes and Job retention remain in force. Recorded actual Provider identity and usage are sensitive operational metadata: Admin-only, no raw prompt, authorization headers, secrets, model output text, plugin stdout or hidden reasoning.

Provider metadata writes are observation, not execution authority; a failed non-critical telemetry write must not retry a model request. The existing Job budget/Step/Job checkpoint transactions remain authoritative. Validity, recovery, pagination, legacy rendering, dark/light styling and VI/EN labels are qualified by the design acceptance tests. The ADR is intentionally limited to the touched Agent Job flow; a repo-wide SQL reorganization is a separate decision.
