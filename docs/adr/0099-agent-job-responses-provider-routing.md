# ADR 0099 — Workload-aware OpenAI LLM routing: Voice Chat, Agent Job Responses

## Status
Accepted for implementation, 2026-10-13.

## Decision
The existing OpenAI-compatible LLM Provider remains the single configured Provider for both workloads, retaining exact source-qualified pin/revision/digest and Voice-capacity admission. **Voice Session always uses the existing `stream()` transport over `/chat/completions`.** Agent Job uses the separate `background_complete()` interface and chooses `/responses` when its pinned OpenAI Provider configuration explicitly declares `supports_responses = true`. Otherwise Agent Job uses the previous Chat Completions transport.

This is an explicit capability assertion by the Admin, **not automatic inference from a model name** and not speculative probing of arbitrary provider endpoints. A Responses transport error fails the model attempt under the existing Job cancellation/deadline/recovery rules; it does **not** silently retry against Chat Completions. Providers of other adapter kinds keep their existing Chat behavior.

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

## Operational safety
- Responses `status=completed` with assistant `output_text` is the only accepted result. Refusal/incomplete/tool calls/empty output or oversized response are never passed to the Planner/Worker as successful content.
- Use the already-pinned Provider pair and normal quota. Do not borrow Voice reserved capacity.
- No remote response-state persistence or polling; long-running reasoning still respects the Job step deadline and cancellation. Remote ambiguous outcomes must not be auto-replayed.
- The Provider API base URL must end at the root version path (for example `https://api.openai.com/v1`), not `/chat/completions`; Requests derive `/responses` from that root.

## Deferred
Model-specific reasoning effort, provider-native repair chains, prompt caching analytics and Responses `background:true` polling are not needed for this change. Native strict schema for model-authored plugin arguments requires a separate Plan Proposal contract decision.
