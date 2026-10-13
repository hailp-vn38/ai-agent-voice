# Agent Job Platform v1.5.1 — executable Python Plugin Store

**Codebase baseline:** `hailp-vn38/ai-agent-voice`, branch `integration/voice-reminders-direct-tts` (as reviewed 2026-10-13).

**Status:** P0 foundation and P3 source integration are implemented: managed LLM A/B, validated DAG execution, fenced Voice submission, Admin result/retry and checked artifact streaming. Production execution requires the managed ProviderRuntimeManager and an admissible provider pair; a deployment missing the backend still returns `runtime_unavailable`. Real Python/Voice/LLM/browser E2E and Linux sandbox qualification remain `NOT_RUN` here and are assigned to another host by the operator. See [P3 evidence and live commands](P3-IMPLEMENTATION.md) and [P0 evidence](P0-IMPLEMENTATION.md).

P1 installation and P2 execution groundwork have interim implementation commits/evidence; their rootless Linux acceptance gates remain open. See [P1 evidence](P1-IMPLEMENTATION.md) and [P2 evidence](P2-IMPLEMENTATION.md).

## Contents

- `docs/agent-job-plugin-platform-codebase-design.md`: canonical v1.5 design and normative acceptance matrix.
- Repository-root [`docs/adr/`](../adr/): canonical ADR 0092–0097. ADR 0092/0095/0096 are accepted architecture with explicit P0 boundaries; 0093/0094/0097 remain proposed. ADR 0093 replaces the previous unimplemented MCP-binding idea; ADR 0095 includes `cancelling` crash recovery.
- `GLOSSARY-JOB-ADDITIONS.md`: additive canonical vocabulary; removes duplicate Job Owner Scope.
- `END-TO-END-WIRING-AUDIT.md`: static producer-consumer trace, integration blockers, load/starvation assessment and test gates.
- `IMPLEMENTATION-PHASES.md`: one canonical rollout checklist with phase-to-phase first-functional-E2E evidence.
- `CHANGELOG-v1.5.md`: v1.4.2 -> v1.5 contract changes and prior reviewer findings closure.
- `PONYTAIL-REVIEW.md`: applied over-engineering review and remaining scope exclusions.

## Canonical decisions

- Plugin is executable, not MCP. Python 3.12 + uv.lock, build-time dependency install, short-lived isolated container worker; packages/files/images version/digest pinned.
- V1 accepts Admin-trusted local ZIP/curated ZIP; no open public marketplace or arbitrary remote installs. Container/runtime/network policy must enforce restrictions; a venv alone is **not** a sandbox.
- One global pair of LLMs and a deployment-wide Background switch; **all admitted Agents can create Jobs** using WS LLM builtin `server_create_job`, with no Agent-specific toggle or grant. Globally ready Plugins are governed by Admin-reviewed package permissions.
- Job-choice snapshots at submit; live Device/Agent revocation checked at worker start; startup terminalizes `cancelling -> cancelled` without erasing unknown outcomes.
- No Job notifications or changes to Reminder/Direct TTS functionality in V1.

## Voice connection and builtin contract

- WS admission registers `server_create_job` as an LLM-visible builtin; **no Job or ToolCall starts on connection alone**.
- On a later LLM ToolCall, `JobEngine::submit` runs asynchronously and returns `created:true,job_id,status:queued` **only after DB commit**; typed `created:false` indicates failure.
- No `agents.background_enabled` field, API or UI toggle. The global `/background` switch still controls whether submissions are accepted; live Device+Agent assignment is checked at submit and worker start.
- No new WS job event, no notification and no synchronous plugin code in Voice Session.

## Rollout

1. P0 uses migration `0105_background_jobs.sql`; canonical ADRs live under repository `docs/adr/`.
2. Implement P0 through P4 as specified in canonical design; keep `/docs/adr/0087–0091` and Reminder migration 0104 unchanged.
3. Validate sandbox on deployment's actual Linux/container host; unit tests on a developer machine do not establish isolation.
4. Use strict design acceptance tests before claiming feature complete.

References: [uv locking/sync](https://docs.astral.sh/uv/concepts/projects/sync/), [Docker rootless](https://docs.docker.com/engine/security/rootless/), [Docker seccomp](https://docs.docker.com/engine/security/seccomp/), [Docker run security/limits](https://docs.docker.com/reference/cli/docker/container/run/).

## Deployment readiness gates

P0: durable queue only; P1: installed plugin only; P2: real Python code and artifacts through a test-only Job/Step seam; **P3: first complete Voice -> Job -> Python -> Admin Web tracer**; P4: rootless/egress/crash/voice-concurrency qualification. A disconnected module or untested Linux policy cannot be described as ready.
