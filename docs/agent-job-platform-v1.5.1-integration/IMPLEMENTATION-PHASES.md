# Implementation phases — v1.5.1 integration-gated rollout

Canonical design: [`docs/agent-job-plugin-platform-codebase-design.md`](docs/agent-job-plugin-platform-codebase-design.md). Baseline: `integration/voice-reminders-direct-tts`; inspect HEAD/migrations before coding. **A phase is complete only with its tested tracer and observable result; compiling files or rendering UI is not enough.**

| Phase | Deliverable and integration to implement | Hard exit gate / next phase dependency |
|---|---|---|
| **P0 foundation** | SQLx migration (after Reminder 0104), Background global settings, source-qualified provider resolution, `JobEngine.submit`/idempotency/immutable snapshot, clock/deadline, startup lock/recovery, single Job scheduler DB scan+notify, shared `AppState` reference. No Agent enable flag. | Admin HTTP submit creates durable `queued` row and scheduler sees it after lost wake; no claim that it can execute until planner/runner implemented. Crash/revoke/idempotency races pass; Reminder baseline stays green. |
| **P1 install** | Privileged ZIP/curated package, Python 3.12 + uv.lock, isolated bounded build (one build permit), digest+permission approval, `disabled/ready/error`, rootless runner preflight, Plugin API. | Real fixture ZIP installs, activates, survives restart, shows frozen choices at submit. Reject zip-slip/oversize/stale lock/build timeout; no host install or uncontrolled network. |
| **P2 execution tracer** | PluginStore.invoke via Job/Step persistence test seam, prepare/start one real Python container, stdin/stdout JSONL, CAS attempt, artifact import, kill/reap and terminal attempt; run `fixture-echo` (network=none). | **Real code executes end to end within engine**: internal integration harness creates one Step and obtains checked artifact. Scheduler may not yet plan automatically because P3 introduces LLM planner. DO NOT label Voice or full Job feature released. |
| **P3 first functional E2E** | LLM A plan + LLM B subagent, DAG step scheduler, terminal Job, end-user Admin `/jobs` list/detail/artifact, Voice `server_create_job` added to admitted ToolDefinition/ToolTarget and async completion fenced to original Tool Round. Integrate the shared submit handle `AppState -> SessionRuntimes -> SessionActor`. | **Single continuous test**: physical or Studio WS hello -> user intent -> real LLM ToolCall -> `{created:true,job_id}` only after SQLite commit -> LLM queued acknowledgement -> scheduler -> Orchestrator -> Python fixture -> Worker -> DB `succeeded` -> Admin Web detail/download. Also test `features.mcp=false`, Silent/DirectTts delivery, stale completion, all Agents eligible, missing tool-capable LLM and failure cases. |
| **P4 deployment qualification** | Rootless runner on target Linux host, enforced networked-crawler egress, saturation/race/crash tests, fail-stop plugin health, cancellation/reaper, terminal cleanup; cross-feature Voice/Reminders load, complete Admin Plugin/Background UI. | All 31 normative acceptance rows, documented commands/results, real deployment preflight. Record queue wait, DB commit, provider wait, container start, resource limits and cancel latency. No full feature claim until rootless and egress tests pass for enabled plugin profiles. |

## Required cross-phase cutlines

- **No undefined bridge:** Every producer and consumer in the canonical wiring matrix must have an owning phase and a test. `POST /plugins/install` returning success, Job `queued` and visible `/background` settings are each necessary but insufficient.
- **No hidden asynchronous guarantees:** The Tool Round must receive matched `JobSubmitCompleted` before speech acknowledgement; WS disconnect after DB commit does not cancel the durable Job. Admin `/jobs` remains the status authority, no new completion WS stream.
- **No circular P0/P1/P2 dependencies:** P0 may create a Job with empty plugin choices / LLM-only requirements or store a queued Job in an explicitly controlled test harness. **Production Background must remain disabled until P3 runtime readiness; a missing planner/worker/scheduler returns `runtime_unavailable`, never a durable stuck Job.** P1 populates installable choices. P2 tests execution through an internal, test-only Step seam; only P3 drives plans automatically. Do not add a production "execute arbitrary plugin" endpoint to make P2 pass.
- **No runtime egress fiction:** Non-network fixture passes P2 with `network=none`. Network crawlers cannot Activate without real enforced egress; P4 must test on deployment host. No cross-host assumptions about Docker rootless or uv caches.
- **No Voice capacity borrowing:** Isolate installer, Background provider and Python-worker permits. SQLite transactions remain short. Expired/revoked/unschedulable Jobs terminalize with typed reason. Preserve Reminder/Direct Speech tests and their 0104 migration.

## Minimum repeatable test recipe (test names are proposed, not existing)

1. Use `scripts/agent-test.sh` for every check. Run targeted Job tests first; then `scripts/agent-test.sh run -- cargo fmt --all --check`, `scripts/agent-test.sh run -- cargo clippy --all-targets --all-features -- -D warnings`, and `scripts/agent-test.sh full`.
2. Use `scripts/agent-test.sh web test`, `scripts/agent-test.sh web typecheck`, and `scripts/agent-test.sh web build` for Admin Web.
3. P2: actual Python container fixture; validate `uv.lock`, uid/capabilities, output/artifact and forced SIGKILL.
4. P3: instrumented real Voice LLM tool calling, worker LLM and DB/browser Job receipt; kill WS and reconnect without replaying the Job.
5. P4: dual deployment-lock conflict, restart recovery, Device mutation vs start race, HTTP egress restrictions and concurrent Voice/Reminder/background load.

**Qualification outcome:** mark each gate `PASS/FAIL/NOT_RUN` with command, platform/image digest and evidence. Never convert proposed tests into a claim they have already passed.

## Current P0 scope

[P0-IMPLEMENTATION.md](P0-IMPLEMENTATION.md) records the foundation tracer and commands. The P0 harness is LLM-only with an immutable empty Plugin choice set. It observes `queued` through the scheduler but never claims planning or Python execution. Actual container-start races, orphan killing, Plugin eligibility and frozen permission tests require P1/P2/P4. Production has no switch to enable the harness.

## Current P3 source scope

[P3-IMPLEMENTATION.md](P3-IMPLEMENTATION.md) records the managed planner/Worker, durable DAG/replan, Voice builtin, Admin retry and streaming implementation, deterministic source gates, and executable live-host tracer. The operator explicitly deferred real rootless/LLM/browser E2E to another host. P3 source work is recorded separately from its hard live exit gate, which stays `NOT_RUN`; deployment acceptance and P4 qualification are not claimed.
