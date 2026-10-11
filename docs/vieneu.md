# VieNeu TTS

`vieneu_http` uses the [VieNeu v3 Turbo streaming API](https://github.com/pnnbao97/VieNeu-TTS/blob/85344322b7258b4e25479b692e8e3396baf9db34/apps/openai_speech.py). It sends text and a voice name to `/v1/audio/speech`, requests PCM16 mono at 24 kHz, and delivers audio while inference is running. It requires `Content-Type: audio/pcm` and `X-Sample-Rate: 24000`, as supplied by the pinned API. The synthesis request has a total timeout and a two-minute audio limit. Synchronous PCM delivery obeys worker backpressure: if a consumer blocks beyond the deadline, synthesis fails when the callback returns; worker cancellation still controls cleanup. Empty, truncated or invalid responses fail the current turn without retrying.

## Start the service

Use the existing production setup from the README, then add the overlay:

```bash
python3 scripts/check-docker-storage.py
docker compose -p voice-agent-prod -f compose.yaml -f compose.prod.yaml -f compose.vieneu.yaml config --quiet
docker compose -p voice-agent-prod -f compose.yaml -f compose.prod.yaml -f compose.vieneu.yaml up -d --build --wait --wait-timeout 1800
```

The first startup downloads model files into the `vieneu-cache` volume. It mounts the image's existing app-owned home so a new volume inherits UID 1000 ownership; Hugging Face files live under `.cache/huggingface` within it. The service uses CPU fp32 with one inference stream and four queue slots. Server worker limits do not increase VieNeu capacity. Tune hardware and upstream concurrency separately if more sessions are needed. The pinned upstream Dockerfile requires network access during build; Hugging Face access is required on first model load. Source pinning does not pin model revisions downloaded by the upstream SDK.

There is no host port mapping. The server reaches `http://vieneu:8000/v1` on the Compose network. To require bearer authentication, set `VIENEU_API_KEY` in the Compose environment and enter the same value in the provider's API key field. Use a trusted private network for HTTP, or an HTTPS endpoint when crossing untrusted networks. The VieNeu process, not the Rust server, owns model inference and memory.

## Configure the provider

In Studio, create a TTS provider with adapter **VieNeu HTTP Streaming** and this desired configuration:

```json
{
  "base_url": "http://vieneu:8000/v1",
  "voice": "Mai Anh",
  "timeout_ms": 30000
}
```

The base URL includes `/v1`. Voice is the exact preset or registered voice name; the upstream `GET /v1/voices` endpoint lists valid names. Language defaults to `vi-VN`. Credentials belong in the separate API key field, never `config_json`. Bind the new Provider Instance to the Agent or its Template and use the TTS test panel to verify audio. A provider can be created and its Rust runtime materialized before VieNeu is online; availability is checked on synthesis.

Studio draft tests require `[provider_runtime]` to be enabled. In legacy startup mode, save and bind the provider, then restart before testing the loaded instance.

If `[provider_runtime]` is enabled, add `vieneu_http = 67108864` to `[provider_runtime.estimated_peak_bytes]` in the deployment config and restart the server before creating the provider. This is a conservative starting budget for the HTTP adapter, not a memory budget for the VieNeu process; remeasure it if worker capacity changes.

For a deployment-configured Provider Instance instead:

```toml
[providers.tts.instances.vieneu_default]
adapter = "vieneu_http"
base_url = "http://vieneu:8000/v1"
voice = "Mai Anh"
timeout_ms = 30000
# api_key = "same-key-as-VIENEU_API_KEY"
```

Set `provider_defaults.tts = "vieneu_default"` to select it as the deployment default. On a server outside Compose, use the VieNeu service's reachable address instead of `vieneu`.

Manage the stack with the same three `-f` files for subsequent commands. `down` retains the model cache; `down --volumes` deletes it. The existing `scripts/compose.sh` commands omit this optional overlay.
