# Agent Job V3 — implementation contract (OpenCode-style)

> Base branch: `feat/agent-job-hybrid-structured-tools`. Accepted 2026-10-14. This document describes the target implementation; code and tests are authoritative for implemented status.
>
> [Runtime flow](flows/10-agent-job-opencode-v3.md) · [ADR 0114](adr/0114-opencode-agent-session-tool-delegation.md) · [Background roadmap](agent-job-opencode-background-roadmap.md)

## Goal and compatibility
Replace V2's Root JSON `delegate/finish` and its `joined_children` prompt field with a real tool-using Root Session. Root calls `subagent(agent,description,prompt,model?,sessionID?,background?)`; Rust creates/resumes a Child Session with specialist instructions and the **full Root-authored prompt**. Child runs its own LLM/tools loop; runtime returns a typed `ChildOutcome` as a Root `ToolResult` paired with the original call ID. Root may follow up via sessionID or call `finish_job` after validation. Provider differences stop at existing Chat/Responses adapters.

Preserve `dag_v1`, persisted `hierarchical_v2`, Voice streaming, reminders, Direct TTS, approved Job Timeline B, immutable provider/plugin snapshots, one-shot Plugin Steps, safe budget/capacity/authority checks, and unknown-outcome no-replay. New submissions use `execution_mode='hierarchical_v3'` **only when native tool qualification and end-to-end acceptance pass**. Existing jobs never change modes.

## Module seams (reuse before adding)
| Existing seam | Required behavior |
|---|---|
| `services/background/agent_loop.rs` | Root and Child Session LLM/tool loops, bounded context, stop/refusal/repair |
| `AgentCoordinator` (one internal module only if needed) | Validate delegation, ownership, depth, allocation, fork/join, ChildOutcome and final gate |
| `database/jobs` | Durable session append/read/claim; Run association; SQLite transactions; metadata-only event emission |
| `providers/llm/mod.rs` | Extend existing `ChatMessage`, `ToolCall`, `BackgroundTurn`, `LlmProvider::background_turn` |
| `models.rs`, worker admission | Exact pinned Root and Child routes; cancellation; deadline; Voice-reserved capacity |
| `plugins/contracts.rs` / Step executor | Frozen tool schema, argument normalization, Step intent, artifact IDs, known/unknown attempt outcome |

Do **not** add `AgentRuntime` facade, `AgentSessionStore` repository trait, `AgentTurnProvider` trait, `ProvenancedOpaqueState` generic interface, provider qualification registry, or context summarization engine.

## Root tools
```json
{"agent":"research","description":"Inspect architecture","prompt":"Investigate the source, verify findings and report missing evidence.","model":null,"sessionID":null,"background":false}
```
`agent` is one of research/document/general and selects specialist system instructions; `description` is a bounded display title; `prompt` is a bounded complete user message (never only goal_excerpt); `model` if present must identify the frozen worker route; `sessionID` is only a reference and requires job+parent ownership; `background=true` returns `background_subagent_not_enabled`.

Native Root tools are `subagent` and `finish_job`. Each native call proposal is checked before dispatch; reject malformed/duplicate IDs, unknown names, arguments not matching source schema, >2 calls, mixed delegate/finish and unauthorized session/model **as a whole batch**. For usable call IDs return matched tool errors. No agent receives a full sibling transcript.

`finish_job` arguments: `summary,findings,evidence_refs,unresolved`. Rust checks no active required children, verifies artifact ownership and coverage, then commits terminal result; model text alone is not evidence of success.

## Child session and continuation
A new session stores Root task prompt as a user message following specialist and runtime-owned system instructions. The Child only sees granted frozen tools. A resumed session retains the original system instructions, provider pin and prior assistant/tool/result history and appends the new Root task prompt as a new user message. Every resume is a new Run under the same Session; concurrent resume must return `session_busy` rather than interleave transcript writes. For ordinary textual tasks, Child may finish with bounded assistant text; do not hard-require `submit_result`. For artifact/business-schema work, keep schema-specific validation and server-owned artifact ID checks.

## Durable SQLite and migration
Add `job_agent_sessions(session_id,job_id,parent_session_id,agent_key,provider_route_json,created_at,updated_at)` and `job_agent_messages(session_id,sequence,kind,payload_json,created_at)`; add nullable `session_id` to existing `job_agent_runs`. Enforce parent+job association, append ordering, session runner exclusion and immutable provenance. Job execution_mode migration must preserve `dag_v1`/`hierarchical_v2`, triggers and indexes. Preserve legacy rows; do not fabricate old sessions. Full transcripts must be access-restricted; metadata-only SSE must not contain task prompts or tool outputs.

Persist accepted tool-call intent before plugin dispatch and matching result after known outcome. A crash after dispatch with unknown result is terminalized using the existing unknown_outcome policy, **not replayed**.

## Provider modes
| Mode | Rollout | Rules |
|---|---|---|
| Native function/tool calling | V3 foreground initial release | Chat and Responses codecs with matched call IDs; exact provider/model/transport qualification |
| Qualified strict-structured decision | Separate later release | Internal action/ID conversion only; never serialize fake native tool messages |
| Text-only completion | Existing tool-free path | Never authorize delegation/plugin execution by parsing free text in production |

Root and Child can use different *pinned* providers. Do not forward Child raw provider tool blocks or reasoning signatures to Root. Keep native replay metadata inside the correct route-specific Adapter. Preflight both pinned routes for native tools; if static Background capacity is zero, reject submission instead of waiting forever. Dynamic contention follows deadline. Never silently switch provider or lower strictness.

## Delivery phases
1. **Baseline:** freeze source branch; add deterministic regression coverage for V2, malformed calls, capacity, Voice and old jobs.
2. **Native codec:** extend existing provider types and tests for complete call batches, matched tool responses, refusals, stopping and final text.
3. **Sessions:** migrate SQLite; implement durable append/read, ownership and resume; Root/Child context construction.
4. **Root tool loop:** `subagent`, `finish_job`, full-batch validation, foreground two-Child fork/join; no `joined_children` JSON.
5. **Child execution + release:** native plugin loop, final assistant text, artifact checks and completion gate; route `hierarchical_v3` only after tests pass.
6. **Optional later:** qualified structured-only provider compatibility. Background Inbox is [separate](agent-job-opencode-background-roadmap.md).

## Canonical acceptance
| ID | Scenario |
|---|---|
| A01 | Root calls subagent with full authored prompt; Child receives specialist/runtime system context |
| A02 | Two child calls in one Root turn; both matched results and one durable join |
| A03 | Child tool A → tool B → final assistant text with persistent matched pairs |
| A04 | Resume owned Child Session; new Run retains history; foreign or busy session denied |
| A05 | Root Chat/Child Responses with isolated provenance and no cross-provider native replay |
| A06 | Chat/Responses two-turn codec roundtrip; additional adapters qualified before enablement |
| A07 | Later structured adapter uses internal IDs, never fake provider-native tool roles |
| A08 | Invalid args/unknown tools/foreign evidence/authority expansion cause no unauthorized Steps |
| A09 | One Child fails; other succeeds; both outcomes delivered; no early Root completion |
| A10 | Missing/duplicate call IDs, over-limit or mixed-action batch rejected as a whole |
| A11 | Restart never auto-replays ambiguous external attempts |
| A12 | Static zero Background slots detected on submit; dynamic contention respects deadline |
| A13 | Provider/plugin revocation and model mismatch fail closed |
| A14 | V2, dag_v1, Voice/Reminder/Direct TTS regressions |
| A15 | REST/SSE Timeline B and artifacts remain consistent and metadata-only |

## Rollback
Stop new V3 submissions, keep immutable mode of existing jobs, preserve interrupted statuses according to restart policy, back up SQLite before migration. Do not execute V3 rows with V2 JSON decision logic.


## Current implementation checkpoint — 2026-10-14

The V3 source implementation on `feat/agent-job-v3-opencode-session-runtime` now includes:

- Native Root `subagent` / `finish_job`, source-schema and whole-batch validation, bounded repair, typed Child outcomes, and foreground two-Child fork/join even when one Child fails.
- Durable Root/Child transcripts, exact authored prompts, specialist/runtime systems, same-Session follow-up with a new Run, and Session-owned evidence across previous Runs without granting another Session's evidence.
- Atomic Child Run allocation, Session claims, task prompts and Root native call intent. A rejected attachment rolls the complete batch back. Migration 0115 enforces immutable Session provenance/transcript content, job/parent/run association and one active Run per Session.
- Ordered, bounded transcripts with matched tool pairs; incomplete external-call intent cannot be continued. Plugin intent is persisted before dispatch; interrupted attempts retain unknown-outcome evidence and are never replayed.
- FK-safe terminal deletion and retention of V3 Runs, messages and Child/Root Sessions; legacy V2/DAG dispatcher paths and rows remain separate.
- Exact-route preflight, static Background-capacity rejection and deadline/cancellation-bounded dynamic admission, retaining Voice reservation.

### Single native executor (2026-10-14)

The native Root/Child session executor is now the only Agent Loop implementation.
Production submissions use it without a deployment attestation field or hierarchical
capability checks. Provider pins, authorization, capacity, frozen operations and
runtime checks remain enforced. Executor functions and session APIs have no version
suffix. The persisted `hierarchical_v3` discriminator remains unchanged for SQLite
compatibility. Runtime dispatch, session allocation, model budgets and terminal
Agent Run updates no longer branch on `execution_mode`. The DAG planner, proposal
compiler and legacy completion executor have been removed. Historical mode values
remain readable; any admitted Job executes the single native session flow.
Restart recovery and unknown-outcome no-replay rules remain unchanged.
Manual retries create new Jobs using the native session executor.

This explicitly supersedes the V2-executor preservation and release-gate portions
of ADR 0103, ADR 0113 and ADR 0114 (durable Agent Sessions), per the current project
request. Existing qualification evidence below describes its original baseline;
it does not establish deployment acceptance for this change.

### Verification and acceptance evidence

All repository checks use `scripts/agent-test.sh`:

```sh
scripts/agent-test.sh targeted services::background voice-agent-server
scripts/agent-test.sh run -- cargo fmt --all --check
scripts/agent-test.sh run -- cargo check --workspace --all-targets
scripts/agent-test.sh full
scripts/agent-test.sh run -- cargo clippy -p voice-agent-server --all-targets -- -D warnings
```

Verification on 2026-10-14, against source baseline `bee57e57` plus this working tree:

| Check | Result | Saved log basename |
|---|---|---|
| Targeted `v3_` | PASS — 22 unit tests + 1 populated migration test | `agent-test.targeted.quo7DY` |
| Formatting | PASS | `agent-test.run.haiX6Q` |
| Workspace/all-targets check | PASS | `agent-test.run.iCeXwq` |
| Server/all-targets Clippy, `-D warnings` | PASS | `agent-test.run.bBjWUd` |
| Workspace tests | PASS — existing explicitly ignored tests remain ignored | `agent-test.full.wsjIVC` |

Logs are retained under the runner-reported temporary `ai-agent-voice-test-logs` directory. Intermediate fixture/compiler/Clippy failures were corrected; their earlier saved logs were not removed. These results apply to local source/fixtures, not deployment qualification.

The deterministic V3 acceptance suite is in `services/background/tests/v3*.rs`, the existing hierarchical/root-contract tests and the Chat/Responses codec tests. `tests/job_v3_migration.rs` upgrades a populated pre-V3 database using the real startup migrator, checks retained V2/DAG modes, retry references, Runs, events, artifacts, indexes and immutable constraints, and verifies `PRAGMA foreign_key_check` is empty. It does not modify the operator's database.

| Acceptance | Local evidence | Deployment boundary |
|---|---|---|
| A01–A02 | Authored prompt, both system instructions, paired results and one join | Real model behavior remains separate |
| A03 | Two real Core operations → final text; persistent matched pairs | External Installer/Runner remains separate |
| A04 | Resume/history, concurrent single-runner claim, foreign/busy/changed-route denial, prior-Run evidence | Deterministic SQLite/runtime coverage |
| A05–A06 | Actual native Chat Root and Responses Child adapters against local HTTP fixtures; two-turn histories stay isolated; malformed/refusal/incomplete codec cases | External provider/model/transport qualification NOT_RUN |
| A07 | Deferred by design | Structured-only rollout remains disabled |
| A08–A10 | Source-schema rejection; unauthorized model/session/background rejects the whole batch; failed Child still joins; Root cannot finish with no Child outcome | Shared plugin schema/evidence/authority regression tests also remain required |
| A11 | Durable unmatched external intent, restart unknown-outcome and denied replay | External Runner interruption qualification remains separate |
| A12 | Static capacity eligibility; dynamic contention respects deadline/cancellation | Deployment admission envelope still requires qualification |
| A13 | Frozen-pin checks and release attestation mismatch denial; shared provider/plugin revocation regression paths | Real revocation during deployed V3 work remains unqualified |
| A14 | V2/DAG and existing Voice/Reminder/Direct TTS regression suite | Device/real-model acceptance is not inferred from host tests |
| A15 | V3 event payloads omit private prompts/tool output; unchanged authenticated REST/SSE projection tests | Live browser/deployment acceptance remains separate |

**Deployment release gates still NOT_RUN:** qualification against actual native Chat/Responses provider routes, models requiring opaque reasoning/replay continuation state, representative operator DB backup/upgrade, external Plugin Installer/Runner and live REST/SSE/Voice acceptance. The current stateless Responses adapter fails closed before tool dispatch if a reasoning item carries opaque replay state (`encrypted_content` or a native reasoning `id`); those models cannot be qualified with this adapter until route-local replay support is implemented. Local HTTP fixtures are codec/runtime evidence, not a claim about an external provider. Anthropic/Gemini native Agent Jobs, structured-only mode and background inbox remain disabled/deferred.

### Known scope limitations

- At most two concurrent Child calls and four total Child Runs by default (configuration-owned budget).
- Every Child uses the immutable worker route; `subagent.model` cannot freely select a third provider.
- Each Child initially receives at most sixteen frozen plugin operations; a larger frozen catalog currently fails closed, pending a specific capability-selection policy.
- The Session transcript is persisted without automatic summarization. Provider-context exhaustion is a bounded error, not automatic tool-pair truncation.
- This checkpoint records source implementation and local acceptance; deployment release qualification remains open.
