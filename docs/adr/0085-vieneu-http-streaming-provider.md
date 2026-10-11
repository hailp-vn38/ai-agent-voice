# ADR 0085 — VieNeu HTTP Streaming Provider

## Status

Accepted. Supersedes the prohibition on Python sidecars and speech HTTP adapters in ADR 0027 for the explicitly selected `vieneu_http` Provider Adapter.

VieNeu v3 Turbo runs as a separately deployed Remote TTS Service. Its existing Python API owns model loading, inference and voice presets; the Rust Provider Adapter owns the HTTP request and validates streamed PCM. This avoids porting the upstream phonemizer, generation scheduler and codec into Rust. The server's HTTP, WebSocket and orchestration still run in one process under ADR 0001. Existing native TTS providers remain available.

SpeechOutput continues to own audio conversion, pacing and playback semantics. The adapter requests raw mono signed little-endian PCM16 at 24 kHz and emits typed float PCM as bytes arrive. Network chunks may split a sample; an incomplete final sample, empty response, unexpected format/rate, excessive output, transport failure or non-success status fails synthesis. Cancellation drops the in-flight HTTP future at a bounded polling interval; an absolute deadline prevents later delivery or successful completion after timeout. A synchronous consumer callback already in progress returns under the existing worker backpressure/cancellation contract before the adapter can finish; no automatic retry occurs. Native worker ownership and cleanup acknowledgement remain unchanged.

HTTP is allowed for an operator-configured service on the deployment network, including a Docker service hostname; HTTPS is supported for other deployments. URL credentials, query strings, fragments and redirects are refused. Optional bearer credentials use the existing write-only Admin credential storage, separate from desired configuration. Voice names are configured explicitly and validated by VieNeu rather than copied into a static voice catalog. The Compose overlay pins the upstream source revision, uses CPU fp32, retains the model cache and exposes no host port. Runtime accounting covers the Rust adapter, while the remote service's model memory and capacity are deployment-owned.
