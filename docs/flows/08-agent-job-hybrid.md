# Flow 08 — Agent Job Hybrid (Root Structured / Child Tools)

Status: implementation branch `feat/agent-job-hybrid-structured-tools`. Related: [implementation contract](../agent-job-hybrid-implementation.md), [ADR 0103](../adr/0103-hierarchical-agent-job-v2.md), [hybrid decision](../adr/0113-hybrid-agent-job-native-tool-calling.md).

## Overview

```mermaid
flowchart TD
    A["Device/Admin submit Job"] --> B["SQLite: pin providers, freeze plugins"]
    B --> C["Scheduler: queued -> planning"]
    C --> D["create_root_run: planning -> running"]
    D --> E["Root LLM: strict JSON delegate / finish"]
    E --> F{"Rust accepts decision?"}
    F -->|No| G["bounded repair or invalid_plan"]
    G --> E
    F -->|Delegate| H["Create 1-2 Child Runs / fork"]
    H --> I["Child LLM: native function tools"]
    I --> J{"Exactly one valid function?"}
    J -->|Plugin| K["Rust: source-schema validate / persist Step / execute"]
    K --> L["Matched tool result + local history"]
    L --> I
    J -->|submit_result| M["Rust: validate Child evidence"]
    M --> N["Finish Child / join fork"]
    N --> E
    F -->|Finish| O["Rust: verified coverage / finish_job"]
```

Rust owns state, permission, quota and execution. An LLM function call is a **proposal**, not authority.

## Submission and durable states

```mermaid
sequenceDiagram
    autonumber
    actor Caller as Admin or Voice tool
    participant DB as SQLite
    participant S as Scheduler
    participant E as Job Engine
    Caller->>DB: submit_job(goal, requester)
    DB->>DB: provider capability preflight and immutable pins
    DB->>DB: freeze available Plugin Choices
    DB-->>Caller: queued job_id
    S->>DB: claim_job
    DB-->>S: planning
    S->>E: run hierarchical_v2
    E->>DB: create_root_run
    DB-->>E: running
```

Legacy `dag_v1` persisted records retain their DAG executor. A retry retains its execution mode. Child cannot spawn another Child; Root can fork at most two concurrently and only after the preceding group is joined.

## Root delegate / finish

```mermaid
sequenceDiagram
    participant E as Rust Engine
    participant R as Root LLM
    participant DB as SQLite
    participant A as Child A
    participant B as Child B
    E->>R: goal + frozen operation names + joined results + strict RootDecision schema
    R-->>E: JSON delegate(tasks)
    E->>DB: validate then agent_decision_accepted
    E->>DB: create_children_for_decision
    par A
        E->>A: own goal + allowed operations
        A-->>E: ChildResult
    and B
        E->>B: own goal + allowed operations
        B-->>E: ChildResult
    end
    E->>DB: finish Child Runs + join_children
    E->>R: bounded children summaries / trusted refs
    R-->>E: JSON finish(result)
    E->>DB: verified_coverage + finish_job
```

Root never receives complete Child transcripts, plugin raw contents or provider call IDs. Rust validates `task_key, role, goal, allowed_operations, evidence_refs, budgets` before advancing.

## Child function loop and protocol mappings

```mermaid
flowchart TD
    A["Child starts"] --> B["Build allowed frozen tools + submit_result"]
    B --> C["LLM required exactly one function"]
    C --> D{"Function"}
    D -->|Unknown / invalid args| E["Reject + bounded diagnostic, no Step"]
    E --> C
    D -->|Allowed plugin| F["Insert persisted Step / Worker Attempt"]
    F --> G{"Known outcome?"}
    G -->|Yes| H["Append assistant function call + matching tool result"]
    H --> C
    G -->|Unknown| I["Fail-stop, no replay"]
    D -->|submit_result| J{"Evidence and result valid?"}
    J -->|No| E
    J -->|Yes| K["Child terminal; join after siblings"]
```

| Transport | Native strict request | Matching local history |
|---|---|---|
| Chat Completions | `response_format.json_schema` for Root; `tools[].function.strict=true` for Child | `assistant.tool_calls[].id` + `role=tool,tool_call_id` |
| Responses | `text.format.json_schema` for Root; `tools[].strict=true` for Child | `output.function_call.call_id` + `input.function_call_output.call_id` |

Child mode sets `tool_choice=required` and `parallel_tool_calls=false`; if upstream rejects these, fail rather than downgrading. Each accepted plugin Step yields a matched Tool Result with its persisted `step_key` and the server-owned artifact UUIDs in `evidence_refs`. Child may cite only evidence from its own succeeded Steps. Invalid `submit_result.evidence_refs` get a precise validation code and allowed IDs for bounded correction, rather than silently re-fetching the same URL. `submit_result` closes only Child and creates no plugin Step. A validated plugin call creates one durable Step using `insert_dynamic_step` and executes through the existing Plugin Runtime.

## Failure / recovery / observation

- Root invalid decision ⇒ `agent_decision_rejected`, bounded repair, then `invalid_plan` if exhausted.
- Child invalid/unauthorized function ⇒ reject whole turn; never partially dispatch multiple calls; tool error result only if the rejected call has a usable ID.
- Known plugin failure ⇒ record failed Step and matched Tool Result, let Child choose next function. Unknown worker outcome ⇒ fail-stop and no replay.
- Provider refusal/incomplete/unsupported strict ⇒ model failure, **no automatic fallback** to Chat JSON text or vendor-managed agent execution.
- Cancel/deadline/revocation and failed-on-restart semantics unchanged.
- Timeline B derives LLM / Root fork / Child tool Step / Child finish / join / Job terminal from durable `job_events` and Job Read View; provider `call_id` is only local Child history.

The canonical acceptance matrix is in [the implementation guide](../agent-job-hybrid-implementation.md); do not duplicate it here.
