# ADR 0117 — Core TTS reuses an exact-version provider and Job Artifact publishing

Status: Accepted for implementation (2026-10-16). Complements [ADR 0114](0114-opencode-agent-session-tool-delegation.md), [ADR 0115](0115-core-plugin-activation-policy.md), [ADR 0116](0116-plugin-value-settings-and-manual-jobs.md).

A new statically bundled Core group `core.tts` exposes `tts.synthesize`, both to V3 Sub-Agents and the existing no-LLM `plugin_operation_v3`. The operation is native Rust, not an External Plugin installation, a new model runner, or ESP32 playback. Admin sets one TTS `provider_ref` via existing versioned Plugin Value Settings; the Job admission transaction separately pins the exact provider ref, incarnation, revision and non-secret configuration digest. A Job may not silently switch to a newer or recreated provider while queued. The provider is reacquired through the existing managed `ResourceLease` for that version.

Synthesis uses the existing long-lived `TtsWorkerRuntime` pool, but reserves `ProviderWorkloadClass::Background` rather than Voice or Diagnostic. The worker must acknowledge cancellation/reset before release; unacknowledged native work is quarantined with the provider ResourceLease retained. The output is a single bounded mono PCM16 WAV (`speech.wav`) published by the existing Step/Artifact transaction, with verified Job ownership and Rust-issued artifact ID. The model sees bounded metadata and evidence refs, never raw samples.

Settings revisions, Core activation revision and Job TTS Provider Pin are independent: disabling Core before a Step starts rejects the start; disabling after a successful start does not retroactively abort synthesis. No new Root/Child messaging, provider override Tool arguments, manual HTTP inference route, output format enum, Artifact database table, or permanent TTS session is introduced.

**Rejected:** a Docker/Python TTS plugin requiring another model installation; borrowing Voice capacity for background inference; pinning only the mutable provider key; diagnostic WAV calls as production generation; arbitrary model-selected voices and providers; releasing a native worker lease on Tokio Future drop without quarantine.

**Consequences:** Background synthesis may be unavailable when all TTS worker capacity is reserved for Voice. Administrators can add available worker capacity through normal provider/runtime configuration. Older Jobs never gain TTS authority from a later Plugin Settings update.

### Implementation clarification (2026-10-18)

The background worker's terminal event is not sufficient to release its provider lease while its native reset is still pending. Cleanup ownership is per physical TTS worker/lease; detached cancellation may defer lease release and quarantine unacknowledged attempts. Temporary Core WAV files use a dedicated `plugins/tts-workspaces` subtree cleaned during startup recovery after deployment locking. A frozen TTS capability alone does **not** make an otherwise safe Agent Job retry unsafe: only a durable started or uncertain TTS attempt does.
