# Flow 10 — Agent Job V3: OpenCode-style sessions, prompts and multi-provider communication

> [Implementation](../agent-job-opencode-implementation.md) · [ADR 0114](../adr/0114-opencode-agent-session-tool-delegation.md). Native foreground flow first. Background notifications are deferred.

## End-to-end
```mermaid
flowchart TD
 A["Admin / Voice submit"] --> B["Immutable Job snapshot and frozen plugins"]
 B --> C["Root Agent Session"]
 C --> D["Root native LLM Turn"]
 D --> E{"Normalized tool proposal"}
 E -->|"subagent"| F["Rust validates full batch"]
 F --> G["Create or resume Child Sessions"]
 G --> H["Child independent LLM / tool loops"]
 H --> I["Typed ChildOutcome and fork join"]
 I --> J["Matched Root ToolResults"]
 J --> C
 E -->|"finish_job"| K["Rust active-child, artifact, coverage gate"]
 K --> L["Commit terminal Job"]
```

## Root -> Child delegation (foreground)
```mermaid
sequenceDiagram
 participant Root as Root Session
 participant Adapter as Provider Adapter
 participant Coordinator as Rust Coordinator
 participant DB as Database
 participant Child as Child Session
 Root->>Adapter: Canonical Messages + [subagent, finish_job]
 Adapter-->>Coordinator: Native subagent ToolCall(id, agent, prompt, sessionID?)
 Coordinator->>Coordinator: Validate complete batch, authority, budget, route
 Coordinator->>DB: Persist delegation intent and child ownership
 Coordinator->>Child: New or resumed independent Session
 Child-->>Coordinator: ChildOutcome(status, summary, artifact_refs)
 Coordinator->>DB: Finish Child Runs; join once
 Coordinator-->>Root: ToolResult(call_id = original id, result = ChildOutcome)
 Root->>Adapter: Replay canonical matched ToolCall / ToolResult
```

## Prompt construction
```text
Child new Session:
  system = specialist role instructions
  system += runtime safety instructions and child-only tool rules
  user = exact Root-authored subagent.prompt
  tools = ONLY pinned/frozen authorized plugin operations
Child resume:
  retain system prefix, prior canonical assistant/tool pairs, exact provider pin
  append user = new Root-authored subagent.prompt
  create new AgentRun linked to same Child Session
```
Child never inherits Root's raw transcript or its siblings' observations.

## Child tool loop
```mermaid
flowchart LR
 S["Canonical Child messages"] --> P["Provider-native adapter"]
 P --> T{"Turn result"}
 T -->|"ToolCalls"| V["Validate whole tool batch against frozen source schema"]
 V -->|"valid"| W["Persist Step intent; execute once"]
 W --> U["Known result/error and artifact IDs"]
 U --> R["Persist matched ToolResult"]
 R --> S
 T -->|"FinalText"| C["Bounded result + evidence checks"]
 C --> O["Runtime ChildOutcome"]
 T -->|"refusal/incomplete"| E["Bounded repair / typed failure"]
 W -->|"unknown outcome"| X["Fail-stop; no replay"]
```

## Provider message adapters
| Canonical | OpenAI Chat | OpenAI Responses | Anthropic | Gemini |
|---|---|---|---|---|
| Assistant ToolCall | assistant.tool_calls/id | function_call/call_id | tool_use/id | functionCall plus native metadata |
| Tool Result | role=tool/tool_call_id | function_call_output/call_id | tool_result/tool_use_id | functionResponse |
| Final text | assistant content | output_text | text block | text part |

Only *actually implemented and qualified* adapters are enabled. Existing code includes Chat and Responses native paths on the feature branch; Anthropic and Gemini require separate implementations and real fixtures. Native thought/reasoning metadata stays within its originating Session route and is never transferred from Child to Root. Root receives only a server-wrapped ChildOutcome.

## Validation and recovery
```mermaid
flowchart TD
 A["Tool call batch"] --> B{"IDs, size, names, schema, authority valid?"}
 B -->|"No"| C["Reject entire batch; no Step and no Child"]
 C --> D["Paired error observations when call IDs usable"]
 D --> A
 B -->|"Yes"| E["Persist call intent"]
 E --> F{"Kind"}
 F -->|"subagent"| G["Child Run then matched ToolResult"]
 F -->|"plugin"| H["Pinned one-shot Step"]
 F -->|"finish_job"| I["No active Child + trusted coverage gate"]
 H --> J{"Outcome known?"}
 J -->|"Yes"| K["Persist result; continue"]
 J -->|"No"| L["Terminal unknown_outcome; never replay"]
```

## Foreground limitations and rollout

Child `submit_result` accepts only `status`, `summary`, `findings`,
`evidence_refs` and `unresolved`. Sending `verified_source_urls` is rejected as
`child_result_server_owned_sources` with a matched ToolResult telling the Child
to remove it; Rust owns source verification. The final rejection is persisted
even when it exhausts the repair budget. A failed ChildOutcome keeps
`error_code=invalid_plan` and includes the final turn's safe `validation_code`
and `validation_path` under `validation`, so Root can target its follow-up.

At finalization, the latest Run of each Child Session determines that Session's
outcome. Earlier Runs and their errors remain in history, and their successful
Steps can still supply verified evidence. A successful resume can recover an
earlier failure; a later failed follow-up or a failed independent Child Session
still prevents success. Source completeness and active-child checks remain in force.

- At most two simultaneous Child calls; no recursive subagents.
- `subagent(background=true)` returns `background_subagent_not_enabled` until a later inbox/notification release.
- `subagent.model` must equal the already pinned worker route. Root and Child may have distinct pinned providers, never dynamic hot-swap.
- `hierarchical_v3` is new-only; persisted V2 and DAG jobs retain existing flows.
- Root cannot finish before all required children resolve, or without verified artifacts/coverage.
- Timeline B continues to derive Job events and Agent Run/Step projections; transcript data is never exposed in SSE.

## Acceptance map
A01 prompt; A02 fork/join; A03 tool chains and final text; A04 resume; A05-A07 provider portability; A08-A10 rejection and completion gate; A11 unknown replay; A12 capacity; A13 pin/revocation; A14 regression; A15 Timeline.


## Grounded single-article summary and URL proof

A real Job with goal "tóm tắt ... https://..." formerly showed five
core.web.extract(mode=page,max_chars=1000) calls scanning navigation;
Child produced a qualified summary, Root compressed it into a potentially
misleading summary, and Job coverage said goal_only_unverified despite success.

The corrected path preserves the same OpenCode-style Session/ToolResult flow:

```mermaid
sequenceDiagram
    participant Root as Root Agent Session
    participant Child as Child Session
    participant Web as Core web.extract
    participant Store as Job Steps + Session Store
    participant Finish as Rust finish_job gate
    Root->>Child: subagent(prompt="Read/summarize URL; keep caveats")
    Child->>Web: mode=article,max_chars<=6000,offset=0
    Web-->>Child: article text, truncated, next_offset, provenance
    Child->>Store: Persist paired ToolCall and visible ToolResult
    opt Article result truncated
        Child->>Web: same URL/mode,next_offset
        Web-->>Child: next article text
    end
    Child-->>Root: ChildOutcome(output,verified_source_urls from own Session)
    Root->>Finish: finish_job(summary,findings,unresolved)
    alt URL unverified/incomplete
        Finish-->>Root: Matched ToolResult source_not_fully_verified
        Root->>Child: subagent(sessionID=old,prompt="Continue the article")
    else Complete server-verified Step provenance
        Finish->>Store: Terminal Job; keep Child summary for single-URL text task
    end
```

**Trust boundary:** verified_source_urls is reconstructed from persisted,
successful core.web.extract observations with matching HTTP status, URL,
declared provenance, continuous offsets and final untruncated text visible
in tool messages. Do not treat a title/HTTP 200/menu text as verified content.
URL evidence is not an artifact ID. The terminal coverage basis for this
narrow case is goal_url_verified (or goal_url_unverified), with requested_url,
verified_source_urls and source_complete on the Job result. A Root rejection
remains one matched ToolResult in its own transcript.

This is source-completeness verification, not semantic truth verification. The
user's text may still contain incorrect claims; content trust and citation
credibility checks are independent policies.


## `finish_job` source and artifact contract

`finish_job` is not an artifact authoring tool. Root sends only `summary`, `findings` and `unresolved`. Rust derives `result.evidence_refs` from persisted Job-owned artifact IDs. Verified URL evidence belongs to `result.coverage.verified_source_urls` and **cannot** masquerade as an artifact or Step key.

```mermaid
sequenceDiagram
    participant R as Root Agent
    participant F as Rust finish_job gate
    participant DB as Job ledger
    R->>F: finish_job(summary,findings,unresolved,evidence_refs=URL)
    F-->>R: Matched ToolResult finish_job_server_owned_evidence + guidance
    R->>F: finish_job(summary,findings,unresolved)
    F->>DB: Resolve server-owned artifact IDs + source coverage
    DB-->>F: No artifact; URL verified from Step provenance
    F->>DB: Finish Job, evidence_refs=[], coverage.verified_source_urls=[URL]
```

Legacy extra fields are **not silently ignored**: the original tool schema rejects them and the normal Root repair budget applies. `ChildResult.evidence_refs` remains separately controlled by the Child's optional completion function and per-Run artifact ownership checks.


## Completing a source-verified Job with unresolved facts

`unresolved` means an **unresolved fact in the article**, not an unfinished
Agent action. For example: "AFC has not yet officially decided whether to
postpone the tournament." This caveat belongs in the user-facing answer even
when the article has been read and summarized successfully.

```mermaid
sequenceDiagram
    participant R as Root Session
    participant C as Research Child
    participant G as Rust completion gate
    R->>C: subagent(prompt=Read and summarize article)
    C-->>R: ChildOutcome(completed, verified_source_urls=[URL])
    R->>G: finish_job(summary,findings,unresolved=[not officially decided])
    G->>G: Verify Child succeeded and URL source_complete=true
    G->>G: Evaluate required sources, capabilities and artifacts
    G-->>R: Terminal succeeded; unresolved preserved as information
```

Root and Child remain separate native tool-calling Sessions. A nonempty
`unresolved` does not override verified completion, whereas incomplete
source provenance, missing mandatory capabilities or a failed required Child
still prevent success. This rule deliberately separates uncertainty **in
knowledge** from an unsatisfied **execution requirement**.


## V3 Job Artifact publication and presentation

A Child's authorized Plugin Operation may write up to 16 files inside the worker `/work/out` directory. Its unchanged stdout schema v1 includes each output basename, MIME and optional size/SHA-256. After the worker is reaped, Rust validates files, copies them in bounded chunks to `data/job-artifacts/<job-id>/.<uuid>.staging`, verifies checksums, atomically renames them and commits artifact rows with the accepted Tool Step. The temporary worker workspace is removed only after that publication attempt. Failed publication leaves no visible artifact descriptor; startup reconciliation removes interrupted orphan files.

The Job Detail read view exposes `id`, `step_key`, `filename`, MIME, size, SHA-256 and creation timestamp, while the existing Step data identifies the Agent Run. Admin downloads still use `GET /api/admin/jobs/{job_id}/artifacts/{artifact_id}`, authenticated with the existing Admin Bearer token. The web Gallery previews bounded images, audio, video and escaped text; large or unsupported files remain downloadable. This does not alter the Root/Child canonical ToolCall/ToolResult matching, Child evidence rules or server-owned `finish_job` evidence generation.

## Opt-in long-running yt-dlp operations

The existing V3 Step/Attempt durable ownership and shared Core/External worker semaphore remain unchanged. To support bounded long media downloads, the Background configuration validator now accepts `step_timeout_seconds <= 1800` and `job_deadline_seconds <= 3600`, with **unchanged defaults** (300s Step / 1800s Job). The Step deadline still cannot exceed the Job deadline. An operator must explicitly raise limits in deployment configuration; no yt-dlp tool silently extends another tool's runtime.

The yt-dlp CLI adapter owns process-group termination and reaping before its Step Future completes. Cancellation is not merely a dropped Tokio Future, since FFmpeg/Deno subprocesses may otherwise outlive the Job. The existing terminal Step/Artifact transaction and Agent Session ToolResult remain authoritative.

## Native document Tool observations (ADR 0126)

The Child can invoke the existing pinned Core `document.convert` and `document.read` operations. Conversion publishes one verified Markdown Job Artifact through the existing Step publisher; only **after** commit does the Child see server-issued `evidence_refs`. The Child can page that Artifact by Unicode-scalar offsets. `observation_truncated` is distinct from `truncated` in the durable page, and neither can prove the whole document has been read. Raw large Tool JSON must not be reproduced inside the terminal coverage summary. Follow the pinned `background.job_output` budgets and existing Root/Child pairing.
