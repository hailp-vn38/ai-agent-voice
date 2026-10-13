# ADR 0099 — Workload-aware OpenAI LLM routing: Voice Chat, Agent Job Responses

## Status
Accepted for implementation, 2026-10-13.

## Decision
The existing OpenAI-compatible LLM Provider remains the single configured Provider for both workloads, retaining exact source-qualified pin/revision/digest and Voice-capacity admission. **Voice Session always uses the existing `stream()` transport over `/chat/completions`.** Agent Job uses the separate `background_complete()` interface and chooses `/responses` when its pinned OpenAI Provider configuration explicitly declares `supports_responses = true`. Otherwise Agent Job uses the previous Chat Completions transport.

This is an explicit capability assertion by the Admin, **not automatic inference from a model name** and not speculative probing of arbitrary provider endpoints. A Responses transport error fails the model attempt under the existing Job cancellation/deadline/recovery rules; it does **not** silently retry against Chat Completions. The `opencode_free` adapter also supports explicit per-model opt-in for the Agent Job transport, while preserving Chat Completions for Voice and for configurations without opt-in.

## Interface and ownership
- `LlmProvider::stream(LlmRequest)`: Voice/diagnostics; no behavior or endpoint change.
- `LlmProvider::background_complete(LlmRequest, Option<Value>)`: Background-only complete text (not a device stream); default adapter collects Chat text without accepting tools; OpenAI adapter may use Responses.
- `JobModels::complete_structured(...)`: Worker calls with executable output schema; non-Responses providers still pass through Rust JSON validation.
- Planner calls with `schema=None` request Responses JSON object mode. Native **strict** JSON Schema is used for Worker output only in this phase: the Plan Proposal currently contains arbitrary plugin `arguments` and cannot truthfully guarantee a universal strict schema without changing its contract. Rust PlanCompiler and bounded repair remain mandatory.
- Responses mode uses `store:false`, `background:false`, `stream:false`, tool-free input, no `previous_response_id` or reasoning-state replay. Token usage is logged as bounded metadata; historical Job metrics are separate work.
- The Rust scheduler, Frozen Plugin Choices, Worker sandbox, immutable Steps, and Job DB remain authoritative. Responses must never invoke vendor-side built-in tools or external MCP.

## Config
Example deployment TOML (no new Agent or Voice toggle):

```toml
[providers.llm.instances.openai-primary]
adapter = "openai"
base_url = "https://api.openai.com/v1"
model = "gpt-5"
supports_responses = true
timeout_ms = 120000
```

For database-configured Provider, set `supports_responses: true` in the OpenAI adapter's JSON configuration. The Admin Provider Catalog exposes the Boolean `Supports Responses (Agent Job only)` field. Default is `false`; existing Providers and Voice stay compatible.

## OpenCode Free opt-in

The existing `opencode_free` Provider now accepts `supports_responses = true` in TOML or `"supports_responses": true` in its Admin JSON config, alongside `model` and `timeout_ms`. The default is `false`, omitted in serialized defaults to preserve existing Provider identities/revisions.

```toml
[providers.llm.instances.zen_free_responses]
adapter = "opencode_free"
model = "muse-spark-1.3-contributor-free"
supports_responses = true
timeout_ms = 60000
```

**Model routing is not universal:** official OpenCode Zen endpoint documentation lists `muse-spark-1.3-contributor-free` under `/zen/v1/responses`, but `nemotron-3-ultra-free` under `/zen/v1/chat/completions`. Do not enable Responses for Chat-only models. For future models, verify the upstream endpoint explicitly before configuring the flag. The free preset still uses its existing fixed `https://opencode.ai/zen/v1` host and `public` credential; successful authenticated access to the Responses endpoint **is not guaranteed** by the presence of that endpoint in the official docs. OpenCode Zen's documented public API typically requires a user's valid Zen API key. Do not assume the no-key Free preset has the same entitlement. Do not add fake OpenCode client headers, placeholder tools or a silent Chat fallback to the Responses request.

The Responses parser, strict schema behavior, response-size limits and error mapping are shared with the OpenAI-compatible Provider implementation. Native Responses never replaces the Voice `stream()` path.

## Operational safety
- Responses `status=completed` with assistant `output_text` is the only accepted result. Refusal/incomplete/tool calls/empty output or oversized response are never passed to the Planner/Worker as successful content.
- Use the already-pinned Provider pair and normal quota. Do not borrow Voice reserved capacity.
- No remote response-state persistence or polling; long-running reasoning still respects the Job step deadline and cancellation. Remote ambiguous outcomes must not be auto-replayed.
- The Provider API base URL must end at the root version path (for example `https://api.openai.com/v1`), not `/chat/completions`; Requests derive `/responses` from that root.

## Deferred
Model-specific reasoning effort, provider-native repair chains, prompt caching analytics and Responses `background:true` polling are not needed for this change. Native strict schema for model-authored plugin arguments requires a separate Plan Proposal contract decision.
