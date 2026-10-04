# Provider Runtime Manager qualification

## ZeroTTS runtime optimization requalification — 2026-10-04

Baseline commit `958aa4e` (`fix(llm): use compatible template switch tool name`), branch `zerotts`.
Scope: one physical ZeroTTS runtime serving many logical voices. See
`docs/zerotts-runtime-optimization-guide.md`.

Workload: `provider-runtime-bench CONFIG tts zerotts_maichi --hold-ms 3000`, installed artifacts
only, one fresh process per observation, `/usr/bin/time -l` alongside the harness' own
`ready_resident_bytes` (`ps -o rss`, sampled while the runtime is `Ready`). Release binaries, Apple
M1 Pro, 16 GiB RAM, `zerotts_onnx` threads = 2. A and B were interleaved in one loop to share
filesystem cache conditions. Five observations each; **these are individual observations, not a
latency distribution, and they are not an SLA.**

Case A is baseline code with `workers.tts.max_workers = 2`, which loaded two ZeroTTS replicas
(eight ONNX sessions) and ran two full-utterance warmups. Case B is the optimized code with the
same `workers.tts.max_workers = 2`: the adapter's declared physical replica count keeps one replica
(four ONNX sessions) while logical width stays 2.

| Metric | A: baseline, 2 replicas | B: optimized, 1 replica |
|---|---:|---:|
| resident runtime load ms (ready) | 8541.503 / 8546.590 / 8698.515 / 8815.583 / 9558.314 | 2579.393 / 2706.426 / 2834.410 |
| artifact prepare ms (full SHA-256 pass) | included in load | 501.459 / 510.552 / 513.963 |
| artifact verify ms | not reported | 496.558 / 505.860 / 508.786 |
| provider contract ms | not reported | 2579.332 / 2706.364 / 2834.341 |
| retained worker session init ms | not reported | 2463.133 / 2580.678 / 2697.804 |
| retained worker warmup ms | not reported | 98.686 / 110.392 / 121.104 |
| total load ms | same as ready | 3080.851 / 3220.389 / 3344.963 |
| resident bytes while Ready | 1073135616 / 1358659584 / 1536442368 / 1549500416 / 2108407808 | 831373312 / 1210761216 / 1218478080 |
| physical replicas / ONNX sessions | 2 / 8 | 1 / 4 |

Load time improves by roughly 3.2x. Case B's resident bytes sit around 1.21 GB; the 831 MB sample is
a run whose resident set had not been faulted back in, so read the group as "about 1.2 GB, sometimes
lower". The case A spread reflects how much of the second replica's memory was still being faulted in
when the sample was taken, so case A is best read as "at least as expensive as its worst observation,
with load time roughly tripled" rather than as a single larger number. Most ZeroTTS weight memory is
file-backed and shared between replicas of the same graph, so replica count shows up far more
strongly in load time and in private arena growth than in instantaneous RSS.

Deployment default (`config.toml`, `workers.tts.max_workers = 1`, one replica both before and after):
baseline ready 4494.056 / 4499.809 / 4662.058 ms at 1280065536 / 1332969472 / 1317076992 resident
bytes; optimized total load 3046.763 / 3146.222 / 3147.119 ms at 1210679296 / 1229340672 / 1229373440
resident bytes. With one replica on both sides, the gain here is the bounded warmup and the removal of
the duplicated model preparation, not the replica count.

Warmup in isolation, from the real-model gate `startup_warmup_is_bounded_and_leaves_no_state_in_the_retained_runtime`:
full-utterance warmup 1819 ms versus bounded warmup 165 ms. One real turn after warmup measured
863 ms, so startup work is not paid back per request.

Inference speed did not regress. `zerotts-core-check`, which exercises production synthesis, codec
decode and canonical Opus, accepted identical output before and after (32 frames, EOA frame 30,
115200 PCM samples) at 20.93 s / 20.26 s baseline and 19.83 s / 19.75 s optimized. The refactor is
outside the production decode loop, and the measured wall time is if anything slightly lower.

Case C (optimized code forced to two physical replicas) is not a supported configuration: the
replica count is an adapter-owned constant with no operator field, so the only comparable two-replica
measurement is case A. The earlier conservative adapter estimates (ZeroTTS 4 GiB) still exceed every
measurement here and were retained; `[provider_runtime]` and the manifest receipt are unchanged,
because this work altered neither the adapter envelope nor model content.

## Refactor requalification — 2026-10-04

The provider descriptor/runtime config refactor changed the manifest to SHA-256
`7bae52392945d9ae8cac091aada1ba32154a8f2fca4a0b0d1251784d4e0a8e36`.
The previous receipt rejected startup. Independent measurements used a private temporary
copy of the deployment configuration with the manager receipt omitted, installed artifacts
only, and one fresh `provider-runtime-bench` process per provider under `/usr/bin/time -l`.
The temporary configuration was deleted after measurement. Configured workers, server-owned
threads and the selected voices were preserved; native warmup and unload acknowledgment
passed for every measured provider. Remote measurements cover client construction only.

| Provider | Readiness ms | Peak RSS bytes | Peak footprint bytes |
| --- | ---: | ---: | ---: |
| Silero | 357.929 | 95,371,264 | 66,568,960 |
| Gipformer | 1,187.427 | 268,959,744 | 246,153,960 |
| Zipformer | 1,862.228 | 347,930,624 | 252,494,544 |
| Kokoro (`diem_trinh`) | 4,596.913 | 1,253,769,216 | 1,386,317,816 |
| ZeroTTS (`maichi`) | 15,126.852 | 1,599,832,064 | 2,439,138,448 |

The existing conservative adapter estimates still exceed each larger measured peak by
at least 25%; they were retained. Zipformer now has a 512 MiB estimate. The 8 GiB
development reservation budget remains unchanged. The deployment receipt was updated
after these measurements. This is one observation per configured provider, not a latency
distribution, an OS memory limit, or a measurement of every selectable voice.
Raw observations and execution settings are in
`.scratch/provider-runtime-manager/evidence/refactor-2026-10-04/`.
The production startup command with the updated `config.toml` reached `/ready` = `ready`
and `/health` = `ok`; SIGTERM shutdown exited 0 with provider resources drained.
`startup.json` records the public probes and exit code. The validation process was stopped
after these probes; no server is left running by this check.
The historical observations below describe the earlier receipt and remain for comparison.

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
