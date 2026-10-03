# Provider Runtime Manager qualification

Development host: Apple M1 Pro, 8 physical/logical CPUs, 16 GiB RAM; macOS 27.0 (26A428). Baseline repository commit: `b3ef953fdeb9f2a83d1513f7f01b700d3fae83a2`. These observations are from the uncommitted implementation, debug binaries, acknowledged installed models and ONNX Runtime configured in this checkout. No remote inference was exercised.

## Native readiness measurements

Workload: `provider-runtime-bench CONFIG KIND KEY`, installed artifacts only, one new process per provider, measured with `/usr/bin/time -l`. VAD has 4 retained workers, Gipformer 2, each TTS 2. Threads per worker: VAD 1, Gipformer 4, Kokoro 1, ZeroTTS 2. Readiness includes artifact verification, retained worker initialization and bounded real inference warmup; successful completion requires native unload acknowledgment. Each row contains **one observation**, not a distribution or an enforced latency target. Artifact pinning was added between observations, so initialization differences cannot be attributed solely to graph removal.

| Provider | Earlier readiness ms | Later readiness ms | Later peak RSS bytes | Later peak footprint bytes |
| --- | ---: | ---: | ---: | ---: |
| Silero VAD | 337.005 | 281.500 | 93,028,352 | 64,619,288 |
| Gipformer | 1,027.018 | 1,029.127 | 330,792,960 | 308,740,912 |
| Kokoro Vietnamese | 5,185.817 | 6,029.159 | 1,754,791,936 | 1,727,023,336 |
| ZeroTTS | 23,577.741 | 17,870.904 | 2,316,550,144 | 2,632,879,296 |

Raw later observations are in `.scratch/provider-runtime-manager/evidence/after-*.stdout` and `.stderr`. Earlier observations are explicitly transcribed in `before-readiness-observations.json`; original raw timing output was not retained. Neither dataset supports p95/p99 claims.

Remote client constructor measurements: OpenAI 0.622 ms / 15,122,432-byte RSS; ChillAudio 3.742 ms / 15,925,248-byte RSS. These cover client/pool construction and cleanup only, **not network, model or inference readiness**.

## Budget contract

`config.toml` activates an 8 GiB **development** manager budget on this 16 GiB host. The remaining memory is for the OS, process/session overhead and other applications. This is a reservation policy, not an OS memory guarantee. Adapter peak reservations round up conservatively from the larger observed RSS/footprint with at least 25% headroom: VAD 128 MiB, Gipformer 512 MiB, Kokoro 3 GiB, ZeroTTS 4 GiB, remote clients 64 MiB. Load concurrency is 1; pending attempts, waiters, resident resources and version metadata have separate bounds. Old active versions and draining/quarantined resources retain reservations, so a new version can be refused rather than overcommitting.

Native requests must remain inside the configured measured model/thread envelope. Zipformer has no estimate because its installed model was not qualified here; it is refused before allocation. Changing hardware, workers, model artifacts or execution settings requires remeasurement. The manifest SHA-256 receipt binds this configuration to the measured artifact manifest and startup rejects a changed receipt. A path-identical native library update requires process restart; do not replace executable/runtime files in a running deployment.

Remote/authenticated backing resources are conservatively isolated across versions because the current secret resolver has no credential generation. Voice/speed settings remain part of native resource identity until an adapter provides a proven operation-scoped selection seam.

## Correctness and evidence

Gipformer one-frame input originally caused an uncaught C++ convolution exception during warmup. Warmup now uses one second of silence, and shorter nonempty utterances receive bounded inference-only silence padding. The ignored real-model public materializer test was explicitly run with 1, 960 and 16,000 samples and passed. Reset clears original utterance state.

Deterministic public seams cover exact SQLite snapshots, WS admission and old/new revision isolation, unbound diagnostics and provenance, singleflight, bounded queues/metadata, shared backing resources with independent quotas, LRU/TTL eviction, native cleanup/quarantine and writer/abort switch boundaries. GET reads cached metadata and does not initialize engines or extend idle lifetime. `/api/admin/system` exposes fixed-cardinality metrics and reservation/usage accounting; it does not expose process RSS as a reservation counter.

The production binary qualification reached `/ready`, prepared and tested exact VAD revision 1,
then exited with `provider_resources_drained=true`; the bounded accounting and aggregate timing
snapshot is in `.scratch/provider-runtime-manager/evidence/managed-startup.json`. The final Rust workspace suite, frontend suite,
frontend typecheck/build, formatting and diff checks passed. The repository's pre-existing strict
Clippy baseline remains recorded separately and is not presented as a clean gate here. Do not
interpret this report as a stable performance SLA or remote-provider qualification.

Native delivery exploration (`tts-vi-v1`, installed-local verification, 1 workload warmup + 5 measured runs) passed for both adapters. Kokoro median processing: 2,537.123 ms, median RTF 0.724892; ZeroTTS: 2,289.048 ms, RTF 0.572262. JSON artifacts are `.scratch/provider-runtime-manager/evidence/tts-delivery-{kokoro_vi,zerotts_maichi}.json`. Their existing percentile fields are empirical summaries of five samples and are not stable tail-latency estimates. The benchmark previously assumed all PCM was 48 kHz; a red/green regression now covers Kokoro 24 kHz, correct audio duration and canonical packet production.
