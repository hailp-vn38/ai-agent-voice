# ADR 0114 — OpenCode-style durable Agent Sessions and Tool Delegation

Status: Accepted for phased implementation, 2026-10-14.
Supersedes the **Root strict JSON decision** portion of ADR 0113 for new hierarchical_v3 Jobs. Does not supersede V2 / historical execution.

## Context
The V2 Root emits JSON delegate/finish decisions and reconstructs joined_children into each subsequent prompt. A Child previously had JSON actions, then native plugin calls, but no durable per-Agent transcript and no reliable matched Root-to-Child delegation tool. Provider-specific tool protocol and history cannot leak into coordinator decisions.

## Decision
1. Root is a native tool-using Agent Session, offering server-owned `subagent` and `finish_job`. The Root writes the Child task `prompt`; Rust validates and starts a separate Child Session. `sessionID` resumes an owned Child Session, appending a new task message and creating a new Agent Run. Child cannot delegate.
2. Foreground delegation accepts up to two `subagent` calls per Root turn, validates the entire batch before any effects, joins even if one child fails, and returns one matched tool result per native call ID. `background=true` is rejected until a separate durable inbox release.
3. Existing `ChatMessage`, `ToolCall`, `LlmProvider::background_turn`, JobEngine, ProviderPin, Plugin Runtime, Job ledger and Timeline B are reused. Database methods under `database/jobs` own sessions, messages and transactional run links. No parallel provider abstraction or separate Agent framework.
4. Child may issue authorized pinned plugin functions or finish with assistant text. Rust, not text, determines terminal status and artifact ownership. Required source/coverage gates remain authoritative.
5. Existing snapshot pins Root to orchestrator and every Child to worker; an explicit `model` must match the worker route. Native tools are mandatory for initial V3. Strict-structured compatibility is deferred and never silently selected.
6. Persistent canonical assistant tool calls and paired observations are replayed through provider-specific adapters; opaque native metadata remains route-scoped and must never leak from Child to Root.
7. `execution_mode='hierarchical_v3'` identifies new Jobs. `dag_v1` and `hierarchical_v2` keep their prior executors, storage and restart behavior. Restore/replay of ambiguous external worker attempts is forbidden.

## Invariants
- All plugin permissions, provider versions and budgets originate in the immutable Job snapshot.
- No unvalidated model tool call becomes a Plugin Step or Child Run.
- Assistant tool calls have exactly one associated tool result in canonical history before the next provider turn.
- Root cannot finish while any required Child is queued/running, or when coverage/evidence is unsatisfied.
- Full task prompts and tool output remain access-controlled in Session storage, never in Job SSE/event payloads.
- A Session is not a Run; resume creates a new Run while preserving Child context.
- Sessions have single runner ownership; background notifications require a distinct ADR.

## Alternatives rejected
More Root JSON repairs; injecting joined_children into ordinary user prompts; a new LlmProvider wrapper or provider-qualification registry; recursive Agents; automatic model switching; copying provider-native reasoning metadata across providers.

## References
- [Implementation](../agent-job-opencode-implementation.md)
- [V3 flow](../flows/10-agent-job-opencode-v3.md)
- [ADR 0103](0103-hierarchical-agent-job-v2.md)
- [ADR 0113](0113-hybrid-agent-job-native-tool-calling.md)


## Amendment: verified URL summaries (2026-10-15)

The Root-to-Child message transport remains unchanged: Root delegates through the
server-owned subagent tool, Child operates in its own Session, and the Root receives
a matched ToolResult. Source URLs and artifact IDs have **different types of proof**.

- For a single explicit URL summary request, Job completion requires successful,
  source-matched, fully observed core.web.extract Step results. Contiguous Unicode
  offsets from zero through an untruncated end are required; source URL, status,
  provenance and text must agree. A nominal Step success or webpage title alone
  is insufficient. This does not claim semantic/factual verification by a human.
- Rust adds verified_source_urls to the typed ChildOutcome using persisted Steps
  owned by the Child Session, including earlier Runs when the same Session resumes.
  These URLs must never be stuffed into artifact_refs or copied from model text.
- A Root finish_job attempt without complete requested URL evidence is answered by
  a paired ToolResult error, enabling Root to resume the Child. No early Job success.
- For the narrowly defined one-URL, one-Child summary, the Child's source-grounded
  summary is preserved as the final summary instead of rewritten by Root; this
  avoids accidental loss of critical qualifiers such as "not yet decided".
- Core web.extract defaults to article-first extraction and gives explicit
  pagination and model-visible truncation metadata. Pages observed by the model
  must be no larger than the bounded excerpt. Full HTML page mode remains opt-in.

The evidence guard is scoped to one-URL summary tasks; arbitrary multi-URL research,
source credibility scoring, citation ranking and document fact-checking are NOT
implicitly claimed by this amendment. Existing native provider/tool contracts,
permissions, acceptance gates, and deferred Background Inbox remain unchanged.
