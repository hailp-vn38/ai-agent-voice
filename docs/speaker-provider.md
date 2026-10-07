# Speaker Provider CAM++

Create a Provider with `type: "speaker"`, `adapter: "campplus_sherpa"` through the existing Admin API. The server generates its key. Typed config accepts nullable `calibration_profile` and bounded `min_speech_ms` (2000), `target_speech_ms` (4000), `max_window_ms` (6000), with minimum ≤ target ≤ maximum ≤ 6000. Model paths, revisions, downloads and threads cannot be set by Admin JSON. Local `secret_ref` is rejected.

The deployment runtime manager must be enabled and declare a conservative `estimated_peak_bytes.campplus_sherpa` budget. `[runtime.onnx.threads] campplus_sherpa = 1` controls execution width. One extractor serves compatible logical revisions under existing logical, physical and global accounting. GET/capabilities show cached state without model loading. POST `/api/admin/providers/{key}/prepare` prepares the exact desired revision.

Provider details in Admin Web include Prepare and a WAV file diagnostic. POST `/api/admin/providers/{key}/test/speaker` accepts raw `audio/wav`, PCM16 mono 16 kHz, 1–12 seconds, at most 512 KiB including WAV metadata; `If-Match: "<provider revision>"` is required. The extractor processes at most the first six seconds under the configured window bound. This diagnostic reports duration, energy and clipping; it is not a calibrated voiced pass. Silence/clipping, malformed/oversized audio and invalid embeddings are rejected. Success returns duration, RMS, clipping and exact provider revision plus embedding contract provenance; it returns no vector. Missing runtime manager returns unavailable. A timed out or disconnected waiter leaves native work holding its exact Resource Lease and capacity until extraction finishes.

Assets use the official [sherpa-onnx speaker models release](https://github.com/k2-fsa/sherpa-onnx/releases/tag/speaker-recongition-models), CAM++ Chinese/English advanced asset 198893102, installed under `models/Speaker/campplus-198893102/`. The pinned [Rust extractor API](https://docs.rs/sherpa-onnx/1.13.8/sherpa_onnx/struct.SpeakerEmbeddingExtractor.html) supplies the dimension. Embedding-space identity hashes canonical model/extractor/preprocessing/dimension metadata. Vector normalization and matching belong to speaker domain services.

## Binding Speaker to a Template

A Template exposes an **optional** `speaker` slot alongside the four required core slots (`vad`, `asr`, `llm`, `tts`). Bind and unlink it through the same Admin API and UI as a core slot:

- `PUT /api/admin/templates/{key}/providers/speaker` with `{"provider_key": "<speaker provider key>"}` and `If-Match: "<template revision>"`.
- `DELETE /api/admin/templates/{key}/providers/speaker` with `If-Match: "<template revision>"`.

The bound provider must have `type: "speaker"`; binding any other type — or binding a speaker provider under a core slot — fails with `provider_type_mismatch`. Both operations are compare-and-swap on the Template revision, so a concurrent edit returns `revision_conflict`. Deleting a Speaker Provider that is still bound to a Template is refused until the binding is removed.

The `speaker` slot is genuinely optional and has **no deployment default and no implicit fallback**: a Template without a speaker binding runs its Voice Sessions without speaker recognition. A Template that binds a speaker provider whose runtime is not loaded fails closed at session resolution instead of silently dropping the slot. In Admin Web the slot appears with the other bindable types in the Template form and pipeline summary.

The first Template assignment to an Agent becomes its enabled default unless an explicit core binding is broken; absent core slots fall back to the deployment providers, and the optional Speaker slot is never counted against core completeness.

Deterministic qualification-provider tests check transport and ownership; native readiness only checks a finite nonzero result. Neither establishes speaker accuracy, replay resistance, ESP32 cross-device performance or Required qualification. Real model/device evidence remains separately required.

Forward migration0007 rebuilds provider/type-slot constraints atomically through SQLx. Startup uses a dedicated connection with foreign keys disabled only for migrations; the normal pool retains foreign keys enabled. The rebuild checks every FK before committing and preserves existing IDs, bindings and deleted AUTOINCREMENT high-water marks.
