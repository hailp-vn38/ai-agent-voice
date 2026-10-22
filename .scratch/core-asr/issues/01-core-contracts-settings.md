# 01 — Register Core ASR contracts and provider settings

Status: resolved
Type: task
Spec: [Core ASR](../spec.md)

## Work

Register initially disabled `core.asr` and `asr.transcribe`, using a new migration without editing existing migrations. Add the strict input/output schemas, initial revision, `p_core_asr_transcribe` projection, transcription coverage and nonreplayable instructions described in the spec. Add the required `provider_ref` settings schema/digest and declare it in both current-settings and pinned Job-settings resolution. Validate activation against a resolvable enabled ASR provider using existing Settings/activation CAS behavior.

Inspect `tools/core/{catalog,contracts,mod}.rs`, `services/background/plugins/settings.rs`, `app/admin/plugins.rs`, Core activation persistence and their tests. Reuse existing ProviderRef parsing; do not add a provider selection argument or External Plugin installation.

## Acceptance

- Fresh and upgraded databases have ASR disabled and preserve existing Core states.
- Contracts reject unknown fields/provider overrides; output supports empty text.
- Missing/wrong-type/disabled providers cannot activate ASR; disable works without settings.
- Current/pinned settings and CAS conflicts work for ASR; TTS behavior is unchanged.

## Verification

Run affected Core/settings/activation test filters through `scripts/agent-test.sh targeted <filter>`. Keep new module tests in separate test files as required by repository conventions.

## Completion

Implemented and verified through the shared native ASR path. See [verification](../verification.md), [review](../review.md), and [flow](../../../docs/flows/15-core-asr-v3.md). Real-provider runtime evidence is reported separately as NOT_RUN.
