# Flow 11 — Core Plugin Activation and Agent Job V3

Reference: [ADR 0115](../adr/0115-core-plugin-activation-policy.md), [Flow 10](10-agent-job-opencode-v3.md).

## Mutable policy versus immutable Job authority

Core groups are compiled into the server. Only their current `enabled` flag is mutable. Every new Job freezes its allowed operations from the enabled Core subset and the already-ready External installations. A frozen choice is the **maximum** authority, not permission to bypass a later Core disable. Re-enabling a group never extends existing frozen choices.

```mermaid
sequenceDiagram
 participant Admin as Authenticated Admin
 participant API as Admin PATCH
 participant DB as SQLite
 participant Submit as Job Submission
 participant Root as Root Session
 participant Child as Child Session
 participant Step as Durable Step
 Admin->>API: PATCH core.web enabled=false, If-Match revision
 API->>DB: publication write guard, BEGIN IMMEDIATE, CAS update
 DB-->>API: enabled=false and next revision
 API-->>Admin: updated tagged Core group
 Submit->>DB: publication read guard, BEGIN IMMEDIATE
 DB-->>Submit: enabled Core groups + ready External installations
 Submit->>DB: persist frozen Job choices after capability validation
 Submit->>Root: frozen operations, no disabled Core Web for new Jobs
 Root->>Child: native subagent(prompt, allowed operations)
 Child->>Step: native ToolCall for a frozen Core operation
 Step->>DB: BEGIN IMMEDIATE, verify reserved Step + policy at start
 alt core group disabled before start
     DB-->>Step: core_plugin_disabled (known denial)
     Step->>DB: persist failed Step / finished Attempt
     Step-->>Child: matched ToolResult with denial code
 else core enabled at start
     DB->>DB: reserved -> start_requested
     Step->>Step: bounded in-process invocation
     Step-->>Child: matched ToolResult and verified result
 end
```

## Concurrent disable and Step start

```mermaid
sequenceDiagram
 participant Patch as Admin PATCH
 participant DB as SQLite write serialization
 participant Worker as Core Step
 Worker->>DB: BEGIN IMMEDIATE and check enabled
 alt disable commits before Step's transaction
     Patch->>DB: commit enabled=false
     DB-->>Worker: deny before start_requested
 else Step start commits first
     Worker->>DB: commit start_requested
     Patch->>DB: commit enabled=false
     Worker->>Worker: finish already-authorized invocation
 end
```

The Core Step waiting for semaphore capacity or merely `reserved` has not started. Current enablement must be checked at the actual `start_requested` transaction. Do not recheck at `mark_plugin_started`, during invocation or at result publication.

## Durable Agent Session considerations

Root `subagent` and `finish_job` tools remain unchanged. Child native function definitions derive from frozen, Child-allowed operations, even if a Core group was disabled after Job submission. A now-disabled call still writes a durable failed Step and one canonical Tool Result paired to the original ToolCall ID; no provider-native protocol mutation or message rewriting is needed. When a Child Session resumes, it retains its old frozen authority. A group re-enabled later does not add new operations.

Neither scheduler `job_eligible` nor successful Job finalization should revalidate the live enabled state across all previously frozen Core choices. Such global checks would cancel unrelated Jobs, although their actual Step start remains permissible. Requirements that cannot be satisfied without a disabled operation remain subject to Rust-owned evidence/coverage checks.

## Management contract

- `GET /api/admin/plugins/builtin`: four static groups with `kind=built_in`, `tags=[core]`, `enabled`, `status`, `revision`, runtime and operation descriptions.
- `PATCH /api/admin/plugins/builtin/{id}`: `If-Match: "<revision>"`, `{"enabled":true|false}`; on success increment revision even if enabled is unchanged.
- `404` unknown group, `409` stale revision, `400` malformed header/body, `503` storage/unavailable.
- External `/api/admin/plugins` remains a separate Store installation listing. Core has no ZIP/image, Install, Uninstall or Test Image actions.

## Failure guarantees

A known policy denial is `core_plugin_disabled`, not `unknown_outcome` or `plugin_runtime_error`. No Worker health degradation and no forced Job-wide cancellation. Database failures fail closed and do not manufacture a successful Tool Result. Existing source/coverage gates, operation contracts, output bounds and Core Web Risk 2 deferral remain unchanged.
