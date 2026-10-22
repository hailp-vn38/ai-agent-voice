# 05 — Integrate Admin settings and verify both invocation paths

Status: resolved
Type: task
Blocked by: 04
Spec: [Core ASR](../spec.md)

## Work

Update `PluginManagePanel.vue` to load ASR providers for `core.asr` and TTS providers for `core.tts`, preserving existing settings revisions, saved selections and catalog-unavailable behavior. Keep the existing Settings/Functions/Runs layout. Show relevant provider help and the existing artifact-binding input for transcription. Do not add an upload flow or new endpoint.

Exercise the public Manual Run path with an authorized WAV artifact, persisted result and Runs display; exercise the V3 Child tool projection/result through existing orchestration. Confirm migrated databases, provider drift rejection, import cleanup and cancellation/quarantine behavior using deterministic providers. Update flow/implementation docs to describe implemented behavior and actual verification evidence.

## Acceptance

- ASR dropdown filters enabled ASR providers and TTS retains its existing filtering.
- Settings save/CAS conflict and missing catalog behavior remain usable.
- Manual admission runs without LLM; Child execution uses the same native operation and receives final text.
- Existing TTS, artifact/document input, Voice ASR and Core activation regressions pass.
- Docs distinguish deterministic verification from real-provider evidence and remaining limitations.

## Verification

Run affected Web tests first via `scripts/agent-test.sh run -- npm --prefix apps/admin-web run test -- <affected-test-files>`, then `scripts/agent-test.sh web typecheck`. Run targeted Rust tests before `scripts/agent-test.sh check` and package/workspace suites where appropriate. Never call `cargo test` directly or ignore failures.

Report real Zipformer and Gipformer inference, simultaneous Voice+Background contention and noninterruptible offline cancellation separately. Do not claim these passed without running them on a native-capable host.

## Completion

Implemented and verified through the shared native ASR path. See [verification](../verification.md), [review](../review.md), and [flow](../../../docs/flows/15-core-asr-v3.md). Real-provider runtime evidence is reported separately as NOT_RUN.
