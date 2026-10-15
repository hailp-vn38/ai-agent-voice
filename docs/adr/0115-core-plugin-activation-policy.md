# ADR 0115 — Durable activation policy for built-in Core Plugins

Status: Accepted (2026-10-15). Supersedes only ADR 0114's former statement that built-in Core groups cannot be disabled. Does not supersede Agent Job V3's OpenCode-style durable Session design.

## Context

Agent Job V3 includes four statically compiled Core groups (`core.web`, `core.document`, `core.data`, `core.artifact`). Previously all eight Core Operations were always frozen into new Jobs. An administrator needs to enable/disable an entire group persistently without a ZIP, container image, Plugin Installation or a new tool execution framework.

## Decision

1. Core groups and their operation-to-group mapping are owned by the Rust binary. A small SQLite table stores only `plugin_id`, `enabled`, `revision` and `updated_at`. All four groups default enabled during a forward-only migration; restart never resets an administrator's policy. A missing known group fails closed.
2. The Core management ID (for example `core.web`) is not the frozen `plugin_id`: persisted Core choices continue using `plugin_id: "core"`, `operation: "web.fetch"` and `contract_revision`. Current policy revision is independent from contract revision.
3. New Job admission freezes only currently enabled Core Operations under the existing publication/SQLite transaction. Required sources/capabilities are validated after filtering; old Job snapshots and idempotent replay are immutable.
4. For already-frozen Jobs, a disabled Core group denies **only new Step starts**. The authoritative check is performed inside `database/jobs/execution.rs::request_plugin_start` under `BEGIN IMMEDIATE`, after checking frozen authority and before `start_requested`. A Step accepted before the disable commit is allowed to finish. Disabled Core decisions are known `core_plugin_disabled` Step failures, not `unknown_outcome`, package-health failures or global Job cancellation.
5. Scheduler eligibility, finalization and Child native tool compilation continue validating immutable Core contracts without checking the live enabled flag for all frozen choices; otherwise unrelated active Jobs could be cancelled. An old Child Session may still display a now-disabled tool, whose call yields one matched error Tool Result. Re-enabling does not expand an old Job's permitted tools.
6. The authenticated Admin interface keeps `GET /api/admin/plugins/builtin` and adds `PATCH /api/admin/plugins/builtin/{id}` with the existing quoted `If-Match` convention. Every successful mutation increments revision, including same-value updates. Views retain `kind: built_in`, add `tags: ["core"]`, `enabled`, `status` and `revision`.
7. External Plugin Store installation/activation, Docker runner and Native Function Calling transport remain unchanged. Web presents Core groups as a section on the existing Plugins page, without install/uninstall actions or a new detail route.

## Rejected alternatives

- TOML or in-memory flags: cannot reliably persist admin state and serialize admission with policy changes without another state mechanism.
- Fake rows in `plugin_installations`: conflicts with package/image digest and lifecycle semantics and adds Store-wide special cases.
- Cancelling a Job whenever a frozen but unused Core choice is disabled: breaks V3 durable sessions and unrelated work.
- Per-operation or Agent/Device-scoped permissions: unnecessary policy model expansion at this release.

## Consequences

Core group enablement is a live **start permission**, while a frozen operation choice is an immutable **maximum authority**. The linearization point for an invocation is the Step-start transaction, not tool visibility in the model's context. No new provider adapters, Agent Session types, event bus, migration of existing Job JSON or generic plugin registry are introduced.

Network target/SSRF/DNS/redirect policy (previously deferred Risk 2) is unchanged. `core.web` must not be exposed to untrusted tenants without independent review. Existing bounded output, source provenance, artifact ownership, deadline, cancellation and Rust-owned completion gates remain in effect.
