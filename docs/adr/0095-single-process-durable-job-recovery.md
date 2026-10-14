# ADR 0095 — Single-process durable Job recovery, cancel, and manual new-Job retry

## Status
Accepted architecture, 2026-10-13. Implementation is phased; acceptance is not execution readiness.

## Decision
SQLx SQLite owns Job and Step checkpoints, one budget ledger in `jobs.budget_used_json` and a per-Step one-shot worker attempt journal. Exactly one deployment process may enable Job scheduling on the shared SQLite file. `queued` may resume with exactly pinned configuration. `planning|running` interrupted across crash becomes `failed/interrupted`; started or possibly started attempts retain `unknown_outcome` and never auto-replay. Rust reaps *only its own labeled* orphan containers; if runtime is unavailable, preserve unknown evidence and schedule safe bounded cleanup independently.

**Startup `cancelling` recovery is unconditional and idempotent:** terminalize to `cancelled` in a DB transaction, mark unstarted descendants cancelled/skipped and started ambiguous attempts unknown, release queued capacity/Job install blockers. Orphan workers are signaled/stopped and retain distinct active-worker pins until truly stopped, so package deletion cannot race a still-running orphan. No lost in-memory ACK may trap the Job in `cancelling`. Crash recovery never claims external effects were reverted. Completion/cancellation races use conditional terminal CAS; first committed transition wins.

`POST /jobs/{id}/retry` is Admin-only and creates a new linked Job with fresh current provider/plugin snapshot, and warns for previous uncertain attempts. Previous history immutable; no step/job automatic retry, including safe-read operations. Absolute deadline starts on submit and includes queue waiting. Artifact data on filesystem uses staging, atomic rename and SQLite metadata with orphan repair, not fictitious cross-resource transaction.

## Why / consequences
An in-process scheduler with durable checkpoints is sufficient for a single-node V1; distributed lease coordinator would add complexity. Python worker start and host DB commit are not atomic, so treating attempts as exactly-once would be unsafe. Startup cleanup cannot guarantee worker termination during the server's downtime unless a separate watchdog is enforced. DB status is terminal even when physical effects are uncertain; operators must inspect evidence before manual retry. No Job notification delivery entity in V1.

## Integration clarification v1.5.1
Durably record `start_requested` before the non-transactional container start; interrupted requested/started attempts are potentially executed and never auto-replayed. A lost scheduler notification cannot strand queued work: a bounded periodic DB scan must detect it, including deadline expiration and revoked/unavailable pins.

## P0 implementation boundary
P0 implements the deployment lock and database recovery. Container reaping is P2/P4; manual new-Job retry is P3. Unknown worker identities remain pinned; this binary cannot launch workers.
