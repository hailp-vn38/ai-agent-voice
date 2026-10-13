# P0 durable foundation — implementation and evidence

Baseline: `c6c8861`, branch `integration/voice-reminders-direct-tts`, 2026-10-13.

## Implemented foundation

- Forward-only migration `0105_background_jobs.sql`: global settings, Plugin installation storage, immutable Job contract/snapshot, one Step attempt journal and artifact metadata. Reminder migration 0104 is unchanged.
- `services/background/`: JobEngine admission/read/cancel, canonical Provider Catalog resolution, exact provider incarnation/revision/config digest, deployment limits, runtime ownership and scheduler.
- `AppState.background`: one shared runtime initialized by all production/bootstrap entrypoints after SQLite migration. Companion file lock uses the canonical path of the actually opened database; scheduler closes submission admission and drains in-flight submissions before releasing the lock, including concurrent shutdown callers. In-memory SQLite does not initialize Background.
- Admin bearer/Origin-protected `GET/PUT /background/settings`, `GET/POST /jobs`, `GET /jobs/{id}`, `POST /jobs/{id}/cancel`. Settings use If-Match. Submission rejects forged fields; receipts follow commit. Same scope/key/normalized payload replays with 200; first receipt is 201; conflicting payload is 409.
- BEGIN IMMEDIATE protects scope-bound replay, live Device/Agent validation, frozen request/model/limits choices and bounded queue insertion. Device and Agent AUTOINCREMENT primary keys are non-reusable incarnations; no new Agent Background flag or identity UUID is needed.
- Startup terminalizes planning/running as failed/interrupted and cancelling as cancelled, preserves unknown outcomes, worker identity pins, completed Steps and the budget ledger. Queued work validates original pins, authority and deadline.
- Periodic bounded DB scan plus Notify. `scheduler_seen_at` records the P0 foundation observation. Expired/revoked/configuration-changed work terminalizes rather than waiting indefinitely.
- Existing Device update/enrollment claim, Agent update, Studio Device mutation and resource deletion share publication coordination. Background provider admission cannot use Voice reservation or the single-slot Diagnostic exception.
- ADR 0092–0097 now live in repository `docs/adr/`. Job vocabulary lives in root CONTEXT.md and is linked from GLOSSARY.md.

## Rollout boundary

Production has no executor and no harness switch. It forces the persisted Background switch disabled on startup, rejects enabled settings/submission with `runtime_unavailable`, and never launches a Python worker or LLM plan. A crate-local `cfg(test)` foundation harness permits LLM-only queued Jobs with an immutable empty Plugin choice set. A requested Plugin source/capability is rejected until P1 implements checked choices.

The P0 tracer is **Admin HTTP -> committed queued Job -> periodic scheduler observation -> Admin read**. It is not a terminal Job/Voice/Python tracer. Actual container start-versus-revocation, Plugin digest/grant eligibility, physical orphan reaping, runtime LLM leases, retry/artifact downloads and AppState -> SessionRuntimes -> SessionActor tool injection remain P1–P4. Unknown worker identities block additional harness submissions; no fake reaper clears them.

## Qualification

All commands use scripts/agent-test.sh. Initial red: the new background HTTP test failed on missing route (404). Review then found a submit-vs-shutdown race; its regression failed before the gate/drain fix. Both saved failures are retained by the wrapper. The regression now passes; Standards and Spec re-review pass. The first workspace run exposed a SQLx in-memory URL reconstruction panic (26 failures). The backing path now comes from the opened SQLite database via `pragma_database_list`; 14 targeted System MCP regressions pass, and the workspace gate is rerun below.

| Gate | Command | Result |
|---|---|---|
| P0 foundation including actual two-process file lock | `scripts/agent-test.sh run -- cargo test --quiet -p voice-agent-server --lib services::background` | PASS, 15 tests |
| Production authenticated fail-closed HTTP | `scripts/agent-test.sh run -- cargo test --quiet -p voice-agent-server --test background_http` | PASS, 1 test |
| Strict Clippy | `scripts/agent-test.sh run -- cargo clippy --all-targets --all-features -- -D warnings` | PASS |
| Reminder regression | `scripts/agent-test.sh targeted reminder` | PASS |
| Direct Speech regression | `scripts/agent-test.sh targeted direct_speech` | PASS |
| Workspace | `scripts/agent-test.sh full` | PASS, 735 passed / 11 ignored / 0 failed |
| Formatting | `scripts/agent-test.sh run -- cargo fmt --all --check` | PASS |
| Admin Web regression | `scripts/agent-test.sh web test` | PASS, 55 files / 227 tests |
| Admin Web typecheck | `scripts/agent-test.sh web typecheck` | PASS |
| Real Python/Voice Job E2E/Linux sandbox/load | P1–P4 fixture and deployment commands | NOT_RUN; executor is not implemented in P0 |

Standards and Spec review are recorded in the local P0 ticket. No claim of functioning Background Job execution follows from these foundation tests.

Final source validation logs: `agent-test.full.kZWfAm` (workspace) and `agent-test.run.cIPmR7` (strict Clippy), retained in the wrapper log directory. Historical failures remain `agent-test.run.ZdaZAq` (initial HTTP red), `agent-test.run.pc4Hlk` (shutdown race red), and `agent-test.full.ZXjuhd` (in-memory regression before fix).
