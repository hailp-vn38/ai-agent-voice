# 05 — P4 deployment qualification

Type: task
Status: claimed
Blocked by: 02, 03, 04
Baseline: P3 source implementation

Implement P4 from `docs/agent-job-platform-v1.5.1-integration/IMPLEMENTATION-PHASES.md` and the 31-row canonical acceptance matrix. The operator runs real E2E on another host; keep that evidence separate from source checks here.

## Acceptance

- Dedicated Linux rootless runner and enforced networked-crawler egress pass real isolation, bypass, private IP, redirect and DNS rebinding tests.
- Plugin package faults persist `error` and stop new dispatch; transient upstream failures and cancellation leave health unchanged. Test does not reset `error`; explicit Activate does.
- Startup and terminal cleanup are bounded and preserve uncertainty, retry lineage and artifact safety.
- Concurrent Voice, Reminder and Background load records queue wait, DB commit, provider wait, container start, resource caps and cancellation/reap latency.
- Admin Plugin/Background UI exposes reviewed grants, package and image identity, runtime readiness and safe confirmation flows.
- All 31 acceptance rows have platform, command and PASS/FAIL/NOT_RUN evidence. No deployment-ready claim before the rootless/egress and continuous E2E gates pass.

## Comments

2026-10-13: Source work added bounded terminal retention, orphan workspace cleanup, package-identity fenced fail-stop health, structured latency/resource events and Plugin/Background UI details. Networked plugins still fail closed with `egress_policy_unavailable`; no enforced egress path is implemented or qualified. Real Linux rootless, crawler egress, concurrent Voice/Reminder/Background and full 31-row acceptance remain NOT_RUN here. See `docs/agent-job-platform-v1.5.1-integration/P4-IMPLEMENTATION.md`. Keep this ticket claimed.

2026-10-13: Source gates PASS: targeted retention and package health, 30 Background tests (1 ignored), 4 Plugin tests, Rust format/strict Clippy, full workspace 764 passed/0 failed/13 ignored, Web 229 tests/typecheck/build. Two failed full-suite attempts exposed a nonblocking accepted socket in the unrelated Vieneu HTTP test; the focused socket correction passed its 8-test file and the final full workspace run. Saved logs and commands are in P4-IMPLEMENTATION.md. This evidence does not resolve the target-host gates.

2026-10-13: Spec review found startup cleanup and retention backlog starvation. Cleanup now drains yielding batches after Linux owned-worker reaping; retention schedules the next 100-Job batch one second later while work remains. The 1,001-workspace and 101-Job tests passed. A full-suite failure on macOS revealed shared temporary plugin storage among parallel tests; cleanup now runs only with a configured Linux runner, and its test uses a private root. Final strict Clippy and full workspace PASS: 765 passed, 0 failed, 13 ignored. The 31-row live-host register is recorded as NOT_RUN in P4-IMPLEMENTATION.md. Enforced networked-crawler egress remains unimplemented, so P4 stays claimed.
