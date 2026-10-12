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
- Model: require IDs with `-free` suffix to prevent accidental use of paid models. The descriptor advertises a snapshot of free IDs; future `-free` IDs can be configured directly.
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

Configure the other defaults as normal; this snippet only shows the LLM slot.

## Database / Admin Web

Choose **Providers → Create → LLM → OpenCode Free**. Select a free model, set the timeout if needed and leave the credential field empty. Test a short Vietnamese response from the draft test panel before saving. Create/bind a Template Agent with that provider using the normal flow. No Admin Web-specific hard-coded adapter list is necessary.

Example stored `config_json`:

```json
{"model":"nemotron-3-ultra-free","timeout_ms":60000}
```

The database schema/validator rejects `api_key`, `base_url`, unrecognized fields, paid model IDs, unsafe model IDs and invalid timeouts for this adapter.

## Smoke test and caveats

```bash
curl -N 'https://opencode.ai/zen/v1/chat/completions' \
  -H 'Authorization: Bearer public' \
  -H 'Content-Type: application/json' \
  -d '{"model":"nemotron-3-ultra-free","messages":[{"role":"user","content":"Xin chào, trả lời ngắn."}],"stream":true}'
```

Run a separate end-to-end verification through Admin provider test, Template Agent chat and MCP tool execution. Watch for 401/403, 404/model_removed, 429/free quota, SSE differences, truncation and tool-call incompatibility. No network test is performed at server startup and no requests are retried automatically by this adapter.

Known limitation: the model list in the descriptor is a snapshot, not live remote discovery. For an automatic free-model catalog, add bounded GET model discovery that filters free chat-completion models *with upstream permission*, and cache it independently from the immutable provider descriptor. Do not show paid models in the Free adapter.

## Sources

- 9Router: https://github.com/decolua/9router
- OpenCode Zen: https://docs.opencode.ai/docs/zen/
- Current model listing: https://opencode.ai/zen/v1/models
- OpenCode inference API documentation: https://dev.opencode.ai/v2/docs/console/api/inference/
