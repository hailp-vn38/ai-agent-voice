# 02 — Pin ASR providers at Job admission

Status: resolved
Type: task
Blocked by: 01
Spec: [Core ASR](../spec.md)

## Work

Follow `database/jobs/tts_provider.rs` for a separate exact ASR provider pin: canonical ref, incarnation, revision and configuration digest. Persist it transactionally with frozen settings/choice for Agent and Manual Jobs. Reacquire the pinned runtime identity and revalidate under catalog publication synchronization before dispatch. Update relevant submission/manual paths, provider dependency/deletion checks and recovery behavior wherever TTS pin integration establishes an applicable requirement.

Audit retry callers and extend durable-attempt authority checks so started/uncertain ASR is not automatically replayed. A frozen but unused ASR capability alone must not prohibit existing safe retry. Do not generalize unrelated provider pin storage.

## Acceptance

- Config-origin and database-origin references are pinned correctly.
- Missing optional ASR leaves unrelated Agent Jobs usable; required/manual ASR rejects unavailable configuration.
- Queued provider changes, disable/delete/recreate and publication races fail closed without fallback.
- Later settings changes do not rewrite admitted Job settings; legacy Jobs gain no authority.
- Fresh manual idempotency and durable-attempt retry rules remain intact.

## Verification

Use targeted admission/provider-pin/retry tests via `scripts/agent-test.sh`. Include migration tests against existing Core/TTS rows and database integrity checks through the same wrapper.

## Completion

Implemented and verified through the shared native ASR path. See [verification](../verification.md), [review](../review.md), and [flow](../../../docs/flows/15-core-asr-v3.md). Real-provider runtime evidence is reported separately as NOT_RUN.
