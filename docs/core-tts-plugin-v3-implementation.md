# Core TTS Plugin V3 — Implementation record and verification gates

**Reference:** `feat/agent-job-v3-opencode-session-runtime`. **Implementation branch:** `feat/core-tts-provider-plugin-v3`. **Status:** Implementation work committed incrementally; native Rust build/E2E qualification pending.

**Related:** [Flow 13](flows/13-core-tts-v3.md), [ADR 0117](adr/0117-core-tts-provider-pinning.md), [Core activation](adr/0115-core-plugin-activation-policy.md), [Plugin value settings](adr/0116-plugin-value-settings-and-manual-jobs.md), [Job Artifact implementation](agent-job-v3-artifacts-implementation.md).

## Intent and scope

Introduce the statically linked, initially disabled Core group `core.tts` and its single operation `tts.synthesize`. Both OpenCode-style `hierarchical_v3` Sub-Agents and `plugin_operation_v3` Admin Manual Runs invoke the same in-process Core operation. It generates mono PCM16 WAV and returns bounded metadata plus one Job-owned artifact via the existing Step publication path.

Do **not** change voice WebSocket protocol, Root/Child Agent Session protocol, Worker JSONL v1, provider adapter implementations, Job output_format, the existing Plugin Runner, or artifact publication database tables. No Python/Docker TTS runtime, new synchronous inference endpoint, file upload or model-selected provider.

## Actual implementation map

| File/module | Integration |
|---|---|
| `migrations/0126_core_tts_provider_pins.sql` | Seed `core.tts` disabled and add job-scoped exact TTS provider identity table. Existing migrations retained. |
| `tools/core/catalog.rs` | Static management group / operation mapping; `tts.synthesize` is nonreplayable metadata, not mislabeled safe_read. |
| `tools/core/contracts.rs` | Native function `p_core_tts_synthesize`, JSON input schema `{text}`, output WAV metadata schema, `tts_synthesis` coverage. |
| `tools/core/mod.rs`, `services/background/plugins/settings.rs` | Core-only static `provider_ref` Settings Schema, independent immutable Plugin Config revisions and digest. |
| `database/jobs/tts_provider.rs` | Resolve settings at admission, pin canonical TTS ref/incarnation/revision/config digest, fail-closed exact resolve. |
| `database/jobs/submission.rs` | Ordinary Agent Job still works if TTS unavailable; only freeze configured eligible TTS operation. |
| `database/jobs/manual.rs` | Manual TTS Run validates settings and pins provider inside Job admission transaction. |
| `services/background/runtime.rs`, `engine.rs` | Pass the existing managed provider manager through to JobEngine and retain quarantined versioned ResourceLeases. |
| `workers/tts/stream.rs` | `start_background` reserves `ProviderWorkloadClass::Background` using the retained TTS worker pool. |
| `tools/core/tts.rs` | Native invocation owns cancellation, bounded PCM16 encoding, WAV header and temporary file lifecycle. |
| `services/background/execution.rs` | Existing core Step-start gate, then `Invocation`; TTS owns timeout/cancellation; existing `ArtifactData` publication unchanged. |
| `GLOSSARY.md` | Canonical *Job TTS Provider Pin* term; no duplicate glossary patch. |

## External contracts

The settings schema contains exactly one field:

```json
{"provider_ref":"config:tts:my-provider"}
```

A database TTS resource key may be used instead of `config:tts:...`. Settings are written with the existing CAS interface `PUT /api/admin/plugins/core.tts/settings`, independent from activation `PATCH /api/admin/plugins/builtin/core.tts`. Neither access path passes a provider ref in the function arguments.

```json
{"text":"Xin chào. Đây là bản tin hôm nay."}
```

The generated Step output metadata is `filename=speech.wav`, `media_type=audio/wav`, `duration_ms`, `sample_rate_hz` and `size_bytes`. Only the Rust Artifact publisher creates artifact UUIDs and Child-owned evidence refs.

### Bounds

- Core TTS `text` has a hard 3,000-Unicode-character limit, no user-selectable voice or language; existing provider binding selects these.
- One native inference and one file per Step; no hidden automatic retry.
- Maximum 180 seconds of audio; same WAV sample rate throughout; mono and finite samples required. Duration, size and Job Artifact quotas cap disk use.
- `ResourceLease` + Background admission permit survive while native work runs. Voice-reserved slots are never borrowed.
- On cancellation, await worker's terminal acknowledgement within cleanup grace; unacknowledged attempts retain ResourceLease/worker accounting and stop admission as `worker_unreaped`. A dropped native Future conservatively detaches cancellation and keeps its versioned resource lease.
- A partial temp WAV is never an Artifact. Publication only follows success and existing `finish_plugin_success` transaction.

## Existing V3 responsibilities reused

- `Frozen Operation Choice`: `plugin_id=core`, `operation=tts.synthesize`, `contract_revision=1`.
- Core activation gate: durable Step start under existing SQLite transaction.
- Plugin Config Pin: immutable selected settings revision; separate Job TTS Provider Pin records exact provider identity.
- Child native tool and Root-Child message adapter: unchanged.
- Admin `plugin_operation_v3`: one Step, no LLM or Agent Runs.
- Local immutable Job Artifacts, reads, Admin Web gallery and WAV preview: unchanged.
- Existing `PluginManagePanel.vue`: schema-driven settings field; no new plugin detail hierarchy or readiness endpoint.

## Known qualification gates

This branch is a code change, not evidence of successful operation on native TTS hardware. Before merge or deploy:

1. Run `cargo fmt --all -- --check`, `cargo check -p voice-agent-server`, then `cargo test -p voice-agent-server` (confirm package name from workspace manifest).
2. Apply all migrations to a populated copy of an existing SQLite DB and run `PRAGMA foreign_key_check`; verify all four original Core enablement rows unchanged and `core.tts` initially disabled.
3. Configure and enable a TTS provider with strictly more total slots than reserved Voice capacity; verify one Admin Manual Run synthesizes a valid WAV with actual sample rate and appears in Job Detail.
4. Verify `hierarchical_v3` Child tool call and `submit_result` evidence ownership, no extra ToolCall protocol or Agent Session.
5. Exercise TTS provider delete/recreate, settings CAS race and provider revision drift after queueing; verify no fallback to current provider.
6. Execute Voice-vs-Background native contention, cancellation, timeout, non-acknowledging worker quarantine, crash/staging orphan reconciliation, quota and Job download tests.
7. Run `npm --prefix apps/admin-web run typecheck` and Web tests for Plugin settings/Functions/Runs/Artifact Gallery.

Known limitation: no Rust toolchain or repository checkout in the editing runtime, so **Rust compilation and native integration were not verified here**. Review potential borrow/timeout interactions and additional capacity/worker-test fixtures before claiming completion.

## Deferred

MP3/Opus encoding, multiple voices, long-text chunking, TTS-to-device delivery, provider fallback, per-Agent override, streaming partial Artifact view, new media-specific authorization model.

## Post-review implementation amendments (2026-10-18)

1. **Safe retry:** Having `tts.synthesize` in a Job's frozen operation catalog alone does not forbid `retry()`. Deny a safe-read retry only when a durable `tts.synthesize` Step has reached `start_requested`, `started`, `finished` or `unknown_outcome`; `plugin_operation_v3` itself is always retried through fresh manual admission.
2. **Native reset ownership:** Every Background synthesis captures a reset acknowledgement flag from its exact physical TTS worker, not `reset_pending()` across the entire pool. Detached cancellation retains the exact `ResourceLease` until its native lease exits **and** that worker's reset flag clears; unresolved cleanup is quarantined and `worker_unreaped` fails closed.
3. **Capacity:** Static `has_background_capacity()` rejects providers that reserve all workers for Voice. Temporary worker saturation retries `start_background` within the existing deadline/cancellation budget, without claiming Voice capacity.
4. **Recovery workspace:** Temporary Core WAV output is isolated under `<db-parent>/plugins/tts-workspaces/<job-id>/<attempt-id>/speech.wav`, separate from External Plugin workspaces. Startup recovery removes UUID-owned native workspaces after the deployment lock is held and jobs have been recovered, before the scheduler resumes. Only the normal `stage_output` / `finish_plugin_success` pipeline publishes the final Job Artifact.
5. **Activation:** `PATCH /api/admin/plugins/builtin/core.tts` with `enabled=true` requires a current, valid Plugin Value Settings contract and a resolvable TTS `provider_ref`. Runtime capacity remains an execution-time condition. Disabling remains supported without a configured provider.

### Follow-up qualification

Regression tests cover safe-retry authority, invalid TTS activation, restart cleanup isolation, native capacity reservation and per-worker reset receipts. A complete Manual Run and V3 Child ToolCall using a **real** TTS provider, including provider revision drift, native cancellation and WAV playback/download, still require integration qualification on the deployment runtime. Do not label this component production-ready until those scenarios pass.
