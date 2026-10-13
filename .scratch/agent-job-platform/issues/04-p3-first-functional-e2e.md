# 04 — P3 first functional E2E

Type: task
Status: claimed
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
