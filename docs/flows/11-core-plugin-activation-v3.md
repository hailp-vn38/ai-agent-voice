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

## yt-dlp: executable readiness, separate from enablement

The compiled `core.yt_dlp` group adds four `yt_dlp.*` operations; it is a built-in Core group, not an External Store installation. `enabled` remains the CAS-protected administrator choice. `availability` describes verified yt-dlp, FFmpeg/ffprobe and Deno binaries on local persistent storage. These dimensions are deliberately independent.

After an enabled transition the server launches an asynchronous, pinned dependency provision attempt. New Job admission removes yt-dlp operations when the dependency bundle is unavailable, without changing `enabled`. Existing frozen Jobs do not gain tools when availability improves. At Step start the usual `request_plugin_start` policy check remains authoritative; execution then checks the immutable bundle path. Missing binaries fail explicitly with `dependency_unavailable`, and no External package is marked unhealthy.

`GET /api/admin/plugins/builtin` adds optional availability/dependencies only to managed Core groups. The retry control `POST /api/admin/plugins/builtin/core.yt_dlp/dependencies/repair` starts a bounded provision attempt and returns 202. It does not change the policy revision or introduce ZIP installation semantics.

## Document conversion operations (ADR 0126)

`core.document` now includes `document.convert` and `document.read` alongside the unchanged `document.extract` / `document.inspect`. The exact frozen Core choice remains `plugin_id=core`, with no Package or Installation. A disabled Core group prevents newly authorized Step starts; it does not alter an already accepted Step or erase published artifacts. AnyDoc parses local verified input only; no implicit OCR, Knowledge import or Wiki publication.
