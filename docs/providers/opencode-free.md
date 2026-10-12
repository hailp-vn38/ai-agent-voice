# OpenCode Free LLM adapter

## Background

9Router implements an OpenCode Free passthrough under the `oc/` routing prefix. Its upstream is OpenCode Zen `https://opencode.ai/zen/v1`, with a non-secret public bearer identity; it is not a locally hosted model. 9Router itself is not necessary for this server.

OpenCode also documents `https://opencode.ai/inference/openai/v1/chat/completions` as a newer, separate API family with unauthenticated free-chat access. This initial adapter intentionally mirrors 9Router's **Zen** route and uses the existing OpenAI-compatible LLM stream integration rather than introducing another parser. Do not assume the two endpoints have identical rules or model availability.

**Caution:** The `Bearer public` pattern is observed in 9Router and is not a contractual OpenCode Zen authentication guarantee. Direct integration can break if the upstream changes access, headers or quotas. Verify current terms and permitted usage before production deployment.

## Adapter

- Adapter ID: `opencode_free`, provider type `llm`.
- Base URL: `https://opencode.ai/zen/v1` (fixed; no arbitrary outbound destination).
- Chat endpoint: `/chat/completions`, streaming over the existing `llm` crate adapter.
- Authentication: built-in public token, **no user API key and no database credential**.
- Model: a required text input in the descriptor/API schema, with no fixed `ModelOption` catalog. Require IDs with the `-free` suffix; model availability is determined by the upstream API, not by validation.
- Timeout: `1_000..=120_000` ms.
- All existing LLM request, generation-cancellation, timeout and quota boundaries remain in effect.
- Tool calling uses the existing OpenAI-compatible bridge. A free upstream model may not support tools; this is deliberately *not* advertised as a universal capability.

## Deployment configuration

```toml
[provider_defaults]
llm = "opencode"

[providers.llm.instances.opencode]
adapter = "opencode_free"
model = "nemotron-3-ultra-free"
timeout_ms = 60000
```

Configure the other defaults as normal; this snippet only shows the LLM slot. When the provider runtime manager is enabled, its adapter reservation must also be configured (the example uses the same 64 MiB estimate as the other remote providers):

```toml
[provider_runtime.estimated_peak_bytes]
opencode_free = 67108864
```

Restart the server after changing runtime configuration. Without this entry, draft and saved-provider tests return HTTP 503 `provider_config_invalid` during runtime admission, before any upstream request.

## Database / Admin Web

Choose **Providers → Create → LLM → OpenCode Free**. Enter the full free model ID (for example, `space-bunny-free`), set the timeout if needed and leave the credential field empty. Test a short Vietnamese response from the draft test panel before saving. Create/bind a Template Agent with that provider using the normal flow. No Admin Web-specific hard-coded adapter list is necessary.

Example stored `config_json`:

```json
{"model":"nemotron-3-ultra-free","timeout_ms":60000}
```

The database schema/validator rejects `api_key`, `base_url`, unrecognized fields, paid model IDs, unsafe model IDs and invalid timeouts for this adapter.

## Smoke test and caveats

The adapter sends `User-Agent: opencode/1.18.31`, a canonical descending `x-opencode-session` (`ses_` + 12 hex characters + 14 Base62 characters), `x-opencode-client: desktop` and `x-opencode-project: global`. Each request opens a fresh upstream session because `LlmRequest` does not carry conversation identity; upstream cache affinity across turns is not preserved.

Streaming is always enabled through the installed OpenAI-compatible transport. Missing `bash`, `glob`, `grep` and `read` tool declarations are appended to meet Zen's free-tier request requirements, following 9Router PRs [#4105](https://github.com/decolua/9router/pull/4105) and [#4132](https://github.com/decolua/9router/pull/4132). Caller tool declarations are preserved. Appended tools are unavailable placeholders; if the model calls one, the stream fails before it can reach the tool executor.

Run the optional live check through the actual Rust provider:

```bash
cargo test -p voice-agent-server --lib live_space_bunny_streams_through_provider -- --ignored --nocapture
```

Run a separate end-to-end verification through Admin provider test, Template Agent chat and MCP tool execution. Watch for 401/403, 404/model_removed, 429/free quota, SSE differences, truncation and tool-call incompatibility. No network test is performed at server startup and no requests are retried automatically by this adapter.

### Space Bunny live check (2026-10-12)

The Zen model listing advertises `space-bunny-free`; `space-bunny` is not the correct ID and returned HTTP 400. Initial requests without the complete session/tool requirements returned HTTP 403 with either `Bearer public` or a user-supplied key. A subsequent request with all four requirements (versioned User-Agent, canonical session, streaming and the tool quartet) succeeded with `Bearer public`: HTTP 200, streamed `OK` and `[DONE]`. The integrated Rust provider also passed its live test, emitting `OK` and the terminal `Finished` event through the shared LLM bridge. No personal key is stored by this adapter.

These requirements are observed upstream behavior, not a stable API guarantee; upstream changes can cause HTTP 403 again.

Model discovery is unsupported. Enter a current upstream free chat model ID directly; the adapter does not maintain a static catalog.

## Sources

- 9Router: https://github.com/decolua/9router
- OpenCode Zen: https://docs.opencode.ai/docs/zen/
- Current model listing: https://opencode.ai/zen/v1/models
- OpenCode inference API documentation: https://dev.opencode.ai/v2/docs/console/api/inference/
