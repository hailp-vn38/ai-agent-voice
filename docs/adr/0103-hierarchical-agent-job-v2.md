# ADR 0103 — Bounded hierarchical Agent Job loops

Status: Accepted for implementation, 2026-10-14.

## Context
A plan-first immutable DAG cannot decide which URL to fetch based on a search result. The existing worker is one tool-free LLM invocation, not an independent tool-using Agent. A redesign must preserve one-shot plugin attempts, pinned authority, Voice provider reservations and reliable Job history.

## Decision
New Agent Jobs use an immutable `hierarchical_v2` execution mode. Legacy records with `dag_v1` retain the current Plan-first executor. One Root Orchestrator delegates bounded tasks to at most two concurrently active Sub-Agents; each Sub-Agent may repeat approved safe-read operations through a private model/action/observation loop. Children cannot spawn further children. Root sees only bounded typed results and trusted Job evidence.

The Rust server owns validation, SQLite Job/Agent Run/Step state transitions and plugin execution. A single additional `job_agent_runs` table records parent-child membership; fork groups are represented by a shared group ID and one unique `join_completed` event, not another table. Tool calls remain existing one-shot Steps. Every decision LLM invocation is a Turn; no Turn table is introduced. Existing LLM activity and Job SSE events are reused.

One shared Job ledger atomically bounds all child allocations, model calls and plugin attempts. Provider and plugin snapshots remain immutable. Restart interruption retains the previous fail-on-restart and unknown-outcome non-replay policy. No extra Admin/Agent enable switch, remote model state, MCP bridge or generic workflow engine.

## Alternatives
A dynamic global tool loop would flood Root context with raw observations. Recursive agents require unbounded permission and budget propagation. A Fork Group or Turn table would duplicate durable LLM/Step metadata. A vendor tool runner would bypass Rust admission.

## Consequences
The Job read view gains execution-mode and Agent Run metadata. Existing durable event cursor and authoritative REST are sufficient for the Web Timeline B. Legacy DAG planning remains available for persisted Jobs. Backward migration is forward-only and needs a data backup for binary rollback.

## Decision validation diagnostics (2026-10-14)

A transport-successful LLM response is **not** an accepted Agent Decision. Rust validates and journals each Root/Child decision as `agent_decision_accepted` or `agent_decision_rejected`, correlated by `agent_run_id`, `run_id`/`turn_id`, and `attempt_no`. Rejections carry only stable `validation_code` and `validation_path` (no model output, tool arguments, prompts, or secrets). Repair receives the typed diagnostic and a bounded previous-response excerpt only in the next transient LLM input. Chat-only providers keep Rust JSON validation; qualified Responses providers receive a strict Root decision schema. The normal Job/Agent failure code remains `invalid_plan` after the bounded repair budget is exhausted.


## V3 successor (ADR 0114)

The above design governs persisted `hierarchical_v2` Jobs. New `hierarchical_v3` Jobs use session-based native Root `subagent`/`finish_job` calls, with durable canonical messages and independent Child Sessions per ADR 0114. The JSON `delegate|finish` proposal and `joined_children` user payload remain V2-only. Rust-owned plugin execution, bounded budgets, immutable pins, Child depth and one-shot attempt safety remain in force. Historical Job rows are not upgraded in place.
