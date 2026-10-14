# ADR 0094 — Rust-owned bounded DAG with frozen Plugin Choices

## Status
Proposed, 2026-10-13.

## Scope since Agent Job V2

The bounded DAG decision remains valid for persisted `dag_v1` Jobs. New `hierarchical_v2` Jobs use the bounded Root/Child loops in ADR 0103, while retaining Rust-owned authority and frozen Plugin Choices.

## Decision
At submit, Rust commits an immutable Request Contract, exact global LLM Provider pair and **bounded Frozen Plugin Choices**: current reviewed, ready `(plugin_id, package_digest, image_digest, operation, permission_digest)` tuples. Orchestrator LLM sees only this snapshot, proposes typed DAG and bounded replans; Rust validates permissions, schemas, dependencies, budgets and original required sources before persisting Steps. A Sub-Agent is an ephemeral Worker LLM execution, not a registered Agent or separately configured LLM.

Each plugin Step pins one operation and one Python container worker attempt. A plugin can internally paginate within resource bounds and output artifacts; Rust does not supply an MCP cursor RPC or a second plugin operation within that Step. Replan may add Steps from frozen choices or supersede pending Steps **only after all running attempts have terminalized**. Replan and later Admin approvals never expand original choices; current revocation may shrink them. Completed Steps and their arguments/artifacts remain immutable. `job_steps` alone represents current DAG; no parallel `plan_json`.

## Why / consequences
LLM output is not permission to run code. Job-submit snapshot prevents adding a plugin after submit/planning and before replan. Worker LLM or plugin output cannot waive required sources or claim a successful external effect. Rust verifies structured coverage and references to same-Job artifacts; absence of explicit coverage records is reported as unverified. Voice ADR 0060 governs only the Voice Tool Round, not Background Step scheduling.

## Integration clarification v1.5.1
The Job scheduler has periodic SQLite scanning in addition to optional wake signals. Only P3 joins persisted Job admission, LLM planner, Python execution, worker summarization, terminal checkpoint and authenticated Admin result into one functional tracer; a P2 test-only invocation alone is not a complete Job.
