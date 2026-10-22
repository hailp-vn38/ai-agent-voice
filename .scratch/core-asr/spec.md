# Core ASR for Background Jobs

Status: resolved
Date: 2026-10-19

## Agreed scope

Add the statically bundled, initially disabled Core group `core.asr` with one operation, `asr.transcribe`. Admin selects an existing ASR provider through Plugin Value Settings, following `core.tts`. Both V3 Sub-Agents and Admin Manual Runs use the same native Rust execution path.

These settings apply only to Background Jobs. Voice Session provider selection, voice protocol, ASR partial/final semantics and enrollment calibration remain unchanged.

## Contracts

Settings contain exactly one required property, `provider_ref`, with no additional properties:

```json
{"provider_ref":"config:asr:my-provider"}
```

Accept a database ASR provider resource key as well as a config-origin ASR reference, using the existing `ProviderRef` parser. Reject the wrong provider type, missing or disabled providers. Enabling the Core group requires valid current settings and a resolvable ASR provider; disabling does not. Runtime capacity is checked at execution.

Operation input contains exactly one required property:

```json
{"artifact_id":"<artifact owned by this Job>"}
```

Operation output contains exactly one required property:

```json
{"text":"Xin chào"}
```

Expose `p_core_asr_transcribe` to a V3 Child using the existing Core tool projection (`plugin_id=core`, `operation=asr.transcribe`, initial contract revision `1`). Declare transcription capability in the existing operation coverage metadata. Return final text only; whitespace-only recognition becomes `{"text":""}` and succeeds. Transcript is untrusted source content, never instructions. Do not synthesize language, confidence or timestamps: the current `AsrResult` interface only exposes text.

Use the existing Settings, Functions and Runs interfaces:

- `PUT /api/admin/plugins/core.asr/settings`, with existing settings CAS revision.
- `PATCH /api/admin/plugins/builtin/core.asr`, with existing activation CAS revision.
- `POST /api/admin/plugins/core.asr/operations/asr.transcribe/runs`, with the existing manual-run envelope and idempotency handling.

Example Manual Run body:

```json
{
  "arguments": {"artifact_id":""},
  "artifact_bindings": [{
    "argument_pointer":"/artifact_id",
    "source_job_id":"<completed source Job>",
    "source_artifact_id":"<source WAV artifact>"
  }]
}
```

Manual admission requires exactly one binding at `/artifact_id`, imports the authorized source artifact into the new Job, and replaces the placeholder with the new artifact ID before schema validation. Preserve existing ownership, source-status, quotas, rollback, idempotency and recovery rules. A Child references an artifact already owned by its current Job. Never accept paths, URLs, base64 audio or arbitrary cross-Job artifact IDs as operation arguments.

## Audio limits for the initial implementation

- One RIFF/WAVE file per invocation, integer PCM16, mono or stereo, sample rate 8–48 kHz inclusive. Reject other WAV encodings and channel counts explicitly.
- Encoded file size at most the smaller of 20 MiB and the existing per-Job artifact byte quota. Enforce before reading/allocating the whole input, including the Child path.
- Nonempty audio of at most 180 seconds. Validate actual decoded frames, channel alignment and checked arithmetic; do not trust filename, MIME type or advertised WAV sizes alone.
- Downmix stereo by averaging channels, convert to finite f32 samples and resample to canonical mono 16 kHz with existing audio facilities. Cap decoded and resampled allocations, including resampler flush output. Do not silently truncate oversized audio.
- Return at most 16,000 Unicode characters and fit the existing bounded Step/ToolResult output. Fail on overflow rather than returning a successful incomplete transcript.
- Silent samples are accepted; do not apply speaker-enrollment quality gates or introduce VAD segmentation.

Reuse installed `hound` and the existing resampler where suitable. Enrollment's `parse_wav` is deliberately mono 16 kHz with a 12-second limit: do not relax that interface or its security/calibration checks to accommodate ASR.

## Provider identity and admission

Follow the exact-version semantics of Core TTS. Within Job admission, freeze the eligible operation and immutable Plugin Config revision, and separately persist the ASR provider ref, incarnation, revision and non-secret adapter/configuration digest. A new ASR pin table is acceptable; do not redesign existing TTS pins just to add ASR.

Ordinary Agent Jobs can still run other operations when optional Core ASR is disabled or unconfigured. Requests requiring transcription and Manual Runs must fail clearly when unavailable. Old Jobs do not gain ASR authority from later settings changes.

Resolve the pinned provider through the existing Provider Runtime Manager and retain its versioned `ResourceLease`. Revalidate provider identity under the existing catalog-publication synchronization immediately before native dispatch. Provider deletion, disablement, recreation or revision drift fails closed; no fallback to current settings or another provider. Settings changes after admission do not rewrite the Job's frozen settings.

## Native execution and cleanup

Use the existing ASR adapter interface and managed runtime. The current `AsrWorkerRuntime::open` admits Voice work, so a Background execution entrypoint is required. Do not route production transcription through the Diagnostic operation. Choose the smallest runtime extension that provides isolated event delivery and exact worker cleanup accounting without changing Voice callers.

Reserve `ProviderWorkloadClass::Background`; never borrow Voice reservation or acquire an Active Turn permit. Reject a provider with no possible Background capacity; temporary saturation may wait only within the Step deadline and cancellation budget. Pin the recognition stream to one worker across open, bounded PCM pushes and finish.

Use the existing operation deadline for bounded waiting, input processing and inference; avoid blocking async executor threads. Preserve command backpressure. Send no partial transcript to a Child, Voice Session or WebSocket.

On cancellation, timeout, task drop or error, request exact-worker cleanup. Release native capacity and the provider lease only after cleanup/reset acknowledgement. If cleanup cannot be acknowledged within grace, quarantine that native work and retain its resource lease/accounting. In particular, the offline ASR adapter cannot hard-cancel an in-progress native decode. Do not claim immediate physical cancellation or spawn unbounded replacements.

Do not automatically replay a started or uncertain ASR attempt. Use explicit nonreplayable operation metadata rather than a generic safe-read instruction. Follow TTS's durable-attempt retry guard: merely freezing ASR capability must not forbid retry of an otherwise eligible Job that never started ASR. A fresh explicit Manual Run remains possible through normal admission.

Successful output uses the existing Step result transaction and contains no new output artifact. Imported input artifacts retain their existing lifecycle. Do not log raw audio or transcript; retain the normal authorized Job result behavior.

## Implementation map

| Area | Required reach |
|---|---|
| Core catalog/contracts/execution | Group, operation, tool projection, settings schema/digest, dispatch and output validation |
| Migrations and `database/jobs` | Disabled activation row, exact ASR provider pin, Agent/manual admission, dispatch checks and retry authority |
| Plugin settings and Admin handlers | Declared Core value settings, activation validation, Functions/Runs descriptors |
| `services/background/manual_admission.rs` | Explicit ASR artifact-binding admission with existing import guarantees |
| Audio and ASR workers | Bounded WAV conversion, Background capacity, isolated events, stream pinning and acknowledged cleanup |
| Background execution/engine | Cancellation/deadline ownership and retained ResourceLease for unreaped ASR work |
| `PluginManagePanel.vue` | ASR provider dropdown and correct help copy, preserving TTS behavior |
| Tests and fixtures | Deterministic WAVs/providers, admission/ownership/race/capacity/cleanup and Admin regressions |
| Documentation | Implementation/flow record after implementation; add an ADR/domain term only when needed, following domain doc rules |

## Acceptance and verification

1. Migration preserves existing Core states and TTS pins; ASR starts disabled. Settings CAS, provider-type validation and activation are covered.
2. Agent and Manual admission pin exact ASR identity. Changed/recreated providers cannot execute queued work; unrelated Agent Jobs still run without ASR.
3. Manual binding imports one authorized WAV and is idempotent; foreign/missing artifacts, invalid pointer/count and unsupported inputs fail without orphan imports.
4. PCM16 mono/stereo at 16 kHz and noncanonical rates produce valid canonical PCM, including 48 kHz TTS output. Malformed/truncated/empty/oversized WAVs and unsupported encodings fail before inference. Exact duration/output boundaries are covered.
5. Success and silence return only final text through both manual and Child paths, with no Voice wire or history changes.
6. Deterministic workers prove Background reservation, bounded saturation, backpressure, pinned stream ownership, cancellation, task drop, timeout, reset acknowledgement and quarantine. Started/uncertain work is never automatically replayed.
7. Existing TTS and Voice ASR contracts continue to pass. Admin ASR dropdown works and TTS dropdown remains correct.

All checks run through `scripts/agent-test.sh`; targeted checks precede package/workspace checks. Never invoke `cargo test` directly. Save logs and inspect failures selectively; preserve nonzero exit codes. Use deterministic tests for mandatory checks, then report real Zipformer/Gipformer inference and simultaneous Voice+Background testing separately as runtime evidence.

## Excluded

MP3/M4A/Opus, direct file upload, URLs, timestamps, diarization, language overrides, partial streaming, long-file segmentation, VAD, provider fallback, new dependencies, new synchronous inference routes, a separate model installation, and changes to Voice Session provider selection.

## Implementation issues

Work in dependency order; each issue owns its targeted regression tests.

1. [Core contracts and provider settings](issues/01-core-contracts-settings.md)
2. [Exact provider pins and admission](issues/02-provider-pins-admission.md)
3. [Artifact input and bounded WAV conversion](issues/03-artifact-input-wav.md)
4. [Background ASR execution and cleanup](issues/04-background-runtime.md)
5. [Admin integration and end-to-end verification](issues/05-admin-verification.md)

## Implementation record

Implemented on the current branch. All five implementation issues are resolved. See [verification](verification.md), [two-axis review](review.md), and [implemented flow](../../docs/flows/15-core-asr-v3.md). Native model runtime evidence remains NOT_RUN, separately from deterministic acceptance.
