# VieNeu live validation

Real CPU ONNX v3 Turbo, fp32, voice Mai Anh. Upstream source pinned to `85344322b7258b4e25479b692e8e3396baf9db34`.

- VieNeu API base URL: `http://127.0.0.1:18090/v1`
- Isolated voice-agent server: `http://127.0.0.1:18091`
- Runtime, model cache, SQLite, config and credentials: `/mnt/storage/ai-agent-voice/runtime/vieneu-live/`
- VieNeu API key is in the permission-0600 `api-key` file in that runtime directory. It is stored encrypted in the isolated server database.
- Admin API token for this isolated fixture: `docker-test-admin`; Voice Protocol token: `docker-test-voice`.

`vieneu-server.wav` is Vietnamese speech returned by the server Admin TTS diagnostic. `vieneu-websocket.wav` is decoded from the real Opus frames received over a Voice Protocol WebSocket session. `summary.json` and the individual JSON reports hold the measurements.

Re-run the provisioned WebSocket client:

```sh
/mnt/storage/ai-agent-voice/runtime/vieneu-live/venv/bin/python .scratch/vieneu-http-live/websocket_flow.py
```

The WebSocket uses `listen.detect` text and a deterministic qualification LLM. This validates real VieNeu inference, the provider, runtime manager, SpeechOutput conversion/pacing, Opus delivery and client decoding. It does not exercise microphone capture, ASR recognition or an external LLM.

Docker was not started because its active data root is `/var/lib/docker`, outside the required storage location. The pinned upstream API and dependency lock are run directly in a dedicated Python 3.12 environment, with all cache/model/runtime data on `/mnt/storage`.

The real repeated-request check exposed stale idle HTTP connection reuse: Uvicorn closes the connection while the retained worker runtime is paused. The provider fix disables idle pooling without adding retries. Its regression test failed before the fix and passed afterwards.
