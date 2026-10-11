# VieNeu requests fail after an idle connection closes

Type: task
Status: resolved

The live CPU ONNX flow exposed a 502 on the request following a six-second pause. Uvicorn closes idle HTTP connections while the retained worker's current-thread Tokio runtime is paused. The client then reuses the stale connection and returns `RemoteConnection` before reaching inference.

Disable idle connection pooling for this provider. Each synthesis opens a connection; preserve the existing single-attempt, redirect, credential, cancellation and deadline behavior. HTTPS deployments incur a connection/TLS handshake per synthesis.

Validation: the regression test failed before the fix and passed afterwards. The real saved-provider Admin API accepted three requests separated by six seconds, including matching voice/language overrides. A real WebSocket session received Opus and decoded it to WAV. The qualification-enabled workspace suite passed 613 tests with four ignored. Live artifacts are in `.scratch/vieneu-http-live/`.
