# 03 — Admit audio artifacts and normalize bounded WAV input

Status: resolved
Type: task
Blocked by: 01
Spec: [Core ASR](../spec.md)

## Work

Extend the explicit artifact-input allowlist in `services/background/manual_admission.rs` for `asr.transcribe`. Require exactly one `/artifact_id` binding in Manual Runs and preserve authorized import, quota, idempotency, rollback and recovery semantics. At execution, resolve only current-Job-owned artifacts through existing artifact facilities; check encoded-size limits before unbounded reads on both manual and Child paths.

Implement PCM16 RIFF/WAVE validation for one/two channels and 8–48 kHz, with nonempty input, 180-second and allocation bounds. Use installed `hound`, existing downmix/resampling facilities and canonical PCM. Verify actual frames, alignment, resampler flush and checked sizes. Leave enrollment's 12-second mono-16-kHz parser and calibration unchanged. No uploads, URLs or new dependencies.

## Acceptance

- Authorized completed source artifacts are imported exactly once under manual idempotency.
- Missing/foreign artifacts, unsupported pointer/count, raw paths and quota overflow fail safely.
- Mono/stereo, 16 kHz and noncanonical rates including 48 kHz normalize correctly.
- Truncation, malformed headers, empty data, unsupported encoding/channels, misaligned frames and duration/size overflow fail before inference.
- Allocation bounds hold for both encoded and decoded audio; oversized input is never silently truncated.

## Verification

Use small generated WAV fixtures and deterministic sample/rate assertions via targeted `scripts/agent-test.sh` runs. Cover import cleanup and existing document/artifact binding regressions.

## Completion

Implemented and verified through the shared native ASR path. See [verification](../verification.md), [review](../review.md), and [flow](../../../docs/flows/15-core-asr-v3.md). Real-provider runtime evidence is reported separately as NOT_RUN.
