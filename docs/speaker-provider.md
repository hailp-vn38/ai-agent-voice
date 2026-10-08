# Speaker Provider CAM++

Create a Provider with `type: "speaker"`, `adapter: "campplus_sherpa"` through the existing Admin API. The server generates its key. The descriptor exposes bounded optional `min_speech_ms`, `target_speech_ms` and `max_window_ms`; omitting them uses server defaults of 2000, 4000 and 6000 respectively, with minimum ≤ target ≤ maximum ≤ 6000. Model paths, revisions, downloads and threads cannot be set by Admin JSON. Local `secret_ref` is rejected.

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

## Speakers and web enrollment drafts

Admin manages Speaker profiles and their enrollment drafts under `/api/admin/speakers` (see the `07 - Speakers` folder in `docs/api/00-all-apis.postman_collection.json`). A Speaker is a named voice with zero or more per-embedding-space voiceprints; until a voiceprint exists it is **not** a usable candidate and no Template binding is required to create it.

- `GET /api/admin/speaker-recognition` returns the bounded enrollment config and resident runtime capability summary.
- `GET/POST /api/admin/speakers`, `GET/PATCH/DELETE /api/admin/speakers/{key}` implement the profile CRUD. Every mutation is compare-and-swap on the Speaker `revision` through `If-Match`, and successful responses carry `ETag: "<revision>"`. Deleting a Speaker that is still referenced by a voiceprint, an agent candidate or an open draft returns `409 speaker_in_use`; the speaker cap returns `409 speaker_quota_exceeded`.
- `POST /api/admin/speakers/{key}/enrollments` opens a draft pinned to one exact Speaker provider revision (`expected_provider_revision`) and embedding space. Creating a draft never changes an active voiceprint and never auto-grants policy. At most one collecting draft exists per speaker (`409 enrollment_in_progress` carries the existing `enrollment_id`) and the process-wide cap returns `409 enrollment_quota_exceeded`.
- `GET /api/admin/speakers/{key}/enrollments/{id}` resumes a draft; if the draft was opened by another process incarnation and the exact provider revision and embedding space still match, it is repinned to the current runtime and its revision is bumped. Otherwise it fails `409 enrollment_runtime_incompatible`. An expired draft answers `410 enrollment_expired` while its tombstone lives, then `404 enrollment_not_found`.
- `DELETE /api/admin/speakers/{key}/enrollments/{id}` cancels a collecting draft with `If-Match: "<draft revision>"` and releases the quota slot.

Drafts are bounded by `[speaker_recognition.enrollment] ttl_ms`; expired drafts have their sample blobs dropped at startup and on a five-minute sweep, and their rows are removed once the tombstone window passes. Draft inspection and reconciliation expose only metadata: no audio, vector or digest bytes.
