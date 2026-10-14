# Agent Job Hybrid V2 — Implementation Guide

Status: implementation on `feat/agent-job-hybrid-structured-tools`, 2026-10-14. Based on [ADR 0103](adr/0103-hierarchical-agent-job-v2.md) and [Hybrid decision](adr/0113-hybrid-agent-job-native-tool-calling.md). See [Rust execution flow](flows/08-agent-job-hybrid.md) and [Web Timeline B flow](flows/09-agent-job-web-timeline.md).

## Decision

- Root Orchestrator proposes `delegate | finish` via **strict native Structured Outputs** (OpenAI Responses `text.format` or Chat Completions `response_format`).
- Each Child proposes exactly one native **function call** per LLM Run. Available functions consist of frozen, Child-allowed safe-read Plugin Operations plus a Rust-owned `submit_result` function.
- Rust is the only executor: it checks call ID, binding, source operation schema, budgets, credentials/permissions, persistence, and trusted evidence before dispatch.
- `submit_result` closes the Child Run, **not** the Job and **not** a Plugin Step. Root resumes after a fork group joins.
- Voice `stream()`, persisted `dag_v1`, existing SQLite Job/Agent Run/Step/Event schemas, restart/unknown-outcome policy and Timeline B layout remain unchanged.

## Deep-module interfaces and implementation seams

| Interface/seam | Rust implementation | Contract |
|---|---|---|
| `LlmProvider::background_turn(LlmRequest, BackgroundMode)` | `providers/llm/mod.rs` | Strict `Structured` or `Tools`; never silently fall back |
| Chat one-shot | `providers/llm/openai/chat.rs` | `stream:false`, strict schema/functions, `assistant.tool_calls` ↔ `role:tool` with `tool_call_id` |
| Responses one-shot | `providers/llm/openai/responses.rs` | `store:false`, `background:false`, `stream:false`, `function_call` ↔ `function_call_output` with `call_id` |
| Workload admission | `services/background/models.rs`, `workers/llm/mod.rs` | Pinned Provider, shared quota, Voice reservation, cancellation and deadline |
| Child tool binding | `services/background/plugins/contracts.rs` | Compile only frozen + allowed tools; `tool_name` maps to exact operation contract |
| Agent engine | `services/background/agent_loop.rs` | Root schema; bounded Child Function Calling; `submit_result`; no vendor tool execution |
| Durable work | `database/jobs/hierarchical.rs` | Existing `insert_dynamic_step()` and one-shot Worker Attempt; no new DB abstraction |
| Provider preflight | `database/jobs/providers.rs` and `submission.rs` | Root requires `supports_structured_outputs`, Worker requires `supports_function_calling` |

## Provider configuration

Existing `supports_responses` remains endpoint-specific. Add two explicit booleans (default `false`) in both TOML and DB `config_json` for OpenAI-compatible and OpenCode Free adapters:

```toml
supports_responses = false
supports_structured_outputs = true
supports_function_calling = true
```

Each value is an **operator assertion**, not automatically discovered support. Select Root/Worker Provider whose **pinned** model + endpoint accepts the relevant strict mode; `supports_tools()` for Voice is not sufficient. Unqualified adapters fail closed; no automatic Chat-vs-Responses downgrade.

At submission, a missing Root capability returns HTTP **409** `orchestrator_structured_outputs_required`; a missing Worker capability returns HTTP **409** `worker_function_calling_required`. The Job is not created in either case. Provider Text Test alone does not automatically enable these flags.

For `opencode_free`, Chat Voice placeholder tools are not exposed to Background Children. Check endpoint/model-specific entitlement before enabling capabilities.

## Tool contract projection

A small helper projects the available source-schema subset into strict function parameters: object/properties, arrays/items, primitive types and enum. Optional fields are represented as nullable + required by the provider; null is removed only for source-optional fields before Rust validation. Source constraints such as `oneOf`, `not`, string lengths or numeric bounds **remain authoritative in Rust**. Unsupported/non-projectable schema ⇒ operation unavailable; never disable strict mode to make it run.

Reserved function `submit_result` has required fields `status=completed,summary,findings,evidence_refs,unresolved`. Rust validates the final Child result and only accepts evidence IDs belonging to the Job.

## Agent loop acceptance

1. Record LLM Turn, call provider with `tool_choice=required` and `parallel_tool_calls=false`.
2. Reject text, zero/multiple function calls, duplicated IDs, unknown function names, invalid JSON/arguments. Record `agent_decision_rejected` with stable code/path; no Plugin Step. Repair only within existing budget.
3. For a validated plugin function call: `agent_decision_accepted(action=call_plugin)` → `insert_dynamic_step` → `execute_plugin_step_cancel` → matched Tool Result containing bounded output plus **server-owned `evidence_refs` artifact IDs scoped to the persisted Step**. Known plugin failures return a tool result; `unknown_outcome` must fail-stop and **never auto-replay**. Child `submit_result.evidence_refs` are verified against artifacts belonging to that Child's succeeded Steps; URLs and foreign Child references are rejected with `invalid_child_evidence_refs` and an actionable bounded repair hint.
4. For a valid `submit_result`: `agent_decision_accepted(action=finish)`, finish Child Run; join group; return compact typed result to Root.
5. Root validates final coverage and uses existing `finish_job` status transitions.

Only bounded metadata goes into Job Events: `job_id`, `agent_run_id`, `run_id`, `step_key`, action, validation code/path. Do not log prompts, URLs, function arguments, credentials or raw tool results.

## Acceptance tests (single canonical matrix)

| ID | Required scenario |
|---|---|
| T01 | Root delegate two Child Runs → unique fork/join → Root final coverage |
| T02 | Invalid Root decision gets bounded repair, never dispatches unaccepted tasks |
| T03 | Child tool A → tool B → `submit_result`; two durable Steps, matched IDs |
| T04 | Unknown/disallowed tool: no Step/Worker Attempt |
| T05 | Zero/multiple calls or text: reject whole turn |
| T06 | `submit_result` fake/foreign-Child evidence ID: reject with scoped allowed refs; actual Tool Result exposes Step artifact UUIDs |
| T07 | Optional/null projection + source schema `oneOf/not` validation |
| T08 | Capability absent or strict upstream rejection: fail closed |
| T09 | Responses function call ↔ function_call_output `call_id` |
| T10 | Chat assistant.tool_calls ↔ role:tool `tool_call_id` |
| T11 | Worker `unknown_outcome`: no replay |
| T12 | Pin/revision/permission change: no hot swap |
| T13 | Shared LLM/Step/Child budgets, Voice reservation, cancellation and deadline |
| T14 | Voice stream, Reminder, Direct TTS and persisted `dag_v1` compatible |
| T15 | REST/SSE Timeline B stable, one activity per durable Step |
| T16 | OpenCode Free Voice placeholder functions excluded from Child tool catalog |

### Scope and known operational limits

No additional SQLite migration, MCP bridge, network proxy, provider qualification database or new Agent Framework. Native strict JSON/Function Calling depends on provider/model compatibility; **flags do not prove upstream support**. Run the existing Provider Test in **Strict Structured Outputs (Root)** and **Strict Function Calling (Child)** modes against the configured model + transport before production use. Saved and draft Provider Test routes accept optional `mode: structured|tools`; text mode remains backward compatible. Automated tests use deterministic transport fixtures and do not assume internet availability.
