# Ponytail Review — v1.5.1 applied changes

Historical design review before implementation; see [P0 evidence](P0-IMPLEMENTATION.md) for the current foundation. Scope is duplicated or speculative complexity only. Functional wiring defects were handled separately in [`END-TO-END-WIRING-AUDIT.md`](END-TO-END-WIRING-AUDIT.md) and the canonical design acceptance matrix, NOT labeled as Ponytail correctness findings.

1. `docs/agent-job-plugin-platform-codebase-design.md` §7: `delete:` second Voice-only Mermaid sequence repeated the canonical flow in §5. Keep one diagram plus the ToolCall contract.
2. `docs/agent-job-plugin-platform-codebase-design.md` §10: `shrink:` duplicate P0-P4 feature/acceptance matrix competing with standalone phase checklist. Keep only a link; [`IMPLEMENTATION-PHASES.md`](IMPLEMENTATION-PHASES.md) owns phase ordering and tracer gates.
3. `docs/agent-job-plugin-platform-codebase-design.md` §2/§7: `reuse:` reject any invented Job-specific WS reader/queue; inject shared JobEngine submit handle into existing SessionActor and complete through existing sequential Tool Round and typed mailbox event.
4. `docs/agent-job-plugin-platform-codebase-design.md` §4: `native:` no custom Python dependency installer and no runtime dependency updates. `uv sync --locked` only inside the isolated bounded image build.
5. `docs/agent-job-plugin-platform-codebase-design.md` §4/§9: `yagni:` no durable installer-task queue or worker pool. Bounded synchronous installer with one build permit/timeout; one ephemeral container per Step attempt.
6. `docs/agent-job-plugin-platform-codebase-design.md` §10: `shrink:` no additional Job ToolResult transport or Worker callback server; existing builtin ToolResult and Admin HTTP polling are the entire V1 UX.
7. `docs/agent-job-plugin-platform-codebase-design.md` §6: `delete:` separate step attempt/budget ledger and duplicated `jobs.plan_json`. `job_steps` stores a one-shot attempt record and the DAG; `jobs.budget_used_json` stays the budget authority.

**Deliberately retained (not over-engineering):** source+image digest pinning, installation approval, network egress enforcement, actual-start revocation fence, one-shot unknown_outcome, orphan reaper, cancellation recovery and linked E2E regression tests.

**net: -30 to -50 document lines possible** by removing repeated phase table and voice diagram; 0 measured implementation LOC deleted because no Agent Job implementation exists. All added E2E wiring and liveness tests are necessary to close execution gaps, not speculative feature scope.
