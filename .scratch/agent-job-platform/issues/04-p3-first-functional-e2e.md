# 04 — P3 first functional E2E

Type: task
Status: resolved
Blocked by: 02, 03
Baseline: P2 interim source

Implement P3 from `docs/agent-job-platform-v1.5.1-integration/IMPLEMENTATION-PHASES.md` and the canonical design.

## Acceptance

- Orchestrator LLM plans a validated bounded DAG from frozen Plugin choices; Sub-Agent LLM executes Step work; scheduler checkpoints a terminal Job.
- Admitted Device and Studio Voice sessions offer `server_create_job` regardless of `features.mcp`; async submit completion is fenced to its original Tool Round and only acknowledges a committed Job.
- Authenticated Admin `/jobs` list/detail/artifact presents the persisted Job, Step, and checked download.
- Run one continuous Voice ToolCall → queued receipt → planner → Python fixture → Worker → terminal Job → Admin Web tracer, plus stale completion, disabled/capacity/provider, Silent/DirectTts and `features.mcp=false` cases.
- Repeat targeted tests, Rust format/strict Clippy/full workspace, Admin Web tests/typecheck/build, two-axis review and runtime qualification.

## Comments

2026-10-13: Initial Admin result slice: authenticated artifact download route, Job-scoped checked bytes, Job result/plan/lineage fields, Web contract alignment and bounded status polling. The download currently buffers the configured maximum 20 MiB; the specified streaming response remains open. Retry UI was hidden because its API is not yet implemented. This does not satisfy the continuous P3 tracer. Production Background still fails closed with `runtime_unavailable`; P1/P2 rootless Linux container qualification is also open. The user confirmed no rootless host or LLM test configuration is available, so real P3 runtime gates are `NOT_RUN`. Keep the phase claimed until all acceptance gates run successfully.

2026-10-13: Source slice gates: `scripts/agent-test.sh targeted admin_artifact_download_is_authenticated_and_scoped_to_job` PASS; `scripts/agent-test.sh run -- cargo fmt --all --check` PASS; `scripts/agent-test.sh run -- cargo clippy --all-targets --all-features -- -D warnings` PASS; `scripts/agent-test.sh full` PASS (749 passed, 12 ignored); Admin Web test/typecheck/build PASS (228 tests). Two-axis review found no Standards violation and confirmed the P3 Spec gaps above. This is an interim source commit, not phase resolution.

2026-10-13: Operator explicitly requested completion of all P3 source and assigned real E2E execution/fixes to another host. Implemented managed LLM A/B readiness/leases/Background permits, periodic bounded DAG scheduler, durable Step budgets/contracts, quiescent append-only replan, structured provenance/partial policy, cancellation/revocation and deadline handling, shared admitted Voice builtin with bounded typed completion fences, Admin safe-read new-Job retry/idempotency and verified streaming artifact downloads. Public physical WS source tracer confirms no Job at hello, `features.mcp=false` ToolCall/committed receipt, terminal Job through scheduler/Worker and Admin detail after disconnect/reconnect without replay. Live runner/LLM/browser gate remains `NOT_RUN`; details and commands are in `docs/agent-job-platform-v1.5.1-integration/P3-IMPLEMENTATION.md`. The earlier source gaps are addressed; ticket remains claimed for the deferred live hard gate, not for missing implementation.

2026-10-13: Final source handoff gates PASS: Background targeted 29 passed/1 ignored; Voice Job tests 4 passed; ordinary sentence streaming regression passed; Rust format and strict all-target/all-feature Clippy passed; full workspace 761 passed/0 failed/13 ignored; Admin Web 229 tests/typecheck/build passed. Standards and Spec reviews completed with material findings resolved. Detailed log basenames and live host command/environment matrix are recorded in P3-IMPLEMENTATION.md. Source completion does not change the deferred real E2E gate from NOT_RUN.


## Answer

2026-10-13: P3 accepted on the user-authorized isolated deployment on this Linux host. Fixed and reproduced lost terminal checkpoint ownership, unconfirmed LLM EOF acceptance and the live receipt-order verification gap. The counted-dispatch retry/cancel and bounded shutdown/recovery regressions pass; both Standards and Spec reviews have no remaining findings.

The P3-specific P1/P2 prerequisites passed on a dedicated rootless daemon: fixture install/review/Activate, frozen operation execution, checked artifacts and timeout/reap. Eight real LLM/Python cases passed for physical/Studio × MCP false/true × Silent/DirectTts. An additional Voice Job was observed concurrently by Chromium from nonterminal state to success; authenticated detail/download hashes, automatic polling, definite failure/skipped descendants, confirmed new-Job retry and cancellation passed. Browser Studio tickets passed fresh admission, single-use and wrong-origin rejection, and fresh reconnect. Six live Admin HTTP rejection/idempotency cases passed without inserting extra Jobs.

Final source gates: Background 35 passed/1 ignored; Voice Job 4 passed; terminal checkpoint 4 passed; receipt matcher 2 passed; workspace 773 passed/0 failed/13 ignored; strict Clippy and formatting passed; Admin Web 229 tests/typecheck/build passed. Evidence and exact identities/logs: [qualification report](../evidence/p3-qualification-2026-10-13.json) and [P3 acceptance](../../../docs/agent-job-platform-v1.5.1-integration/P3-IMPLEMENTATION.md). P4 and the separate external-dependency/remaining installation qualification criteria are not resolved by this P3 acceptance.
