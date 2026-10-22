# Core ASR verification

Implementation and deterministic acceptance completed. All commands below ran through `scripts/agent-test.sh`; targeted checks preceded workspace-wide execution.

| Check | Result |
|---|---|
| `run -- cargo test --quiet -p voice-agent-server --lib asr_` | PASS, 22 focused tests in the last targeted run; the additional retained-worker reset regression passed in the full run |
| `check` | PASS |
| `run -- npm --prefix apps/admin-web run test -- src/components/plugins/PluginManagePanel.test.ts` | PASS, 3 tests |
| `web typecheck` | PASS |
| `web test` | PASS, 331 tests across 72 files |
| `run -- cargo fmt --all -- --check` | PASS |
| `run -- git diff --check` | PASS |
| `full` | PASS, 1,003 tests across 61 test targets, 15 opt-in tests ignored, no failures |

The full workspace gate includes Core contracts/projection, admission and migrations, existing TTS, Voice ASR, artifact/document input, provider runtime and Admin regressions. The new deterministic tests cover Agent/Manual exact pins, provider drift/recreation and concurrent catalog mutation, frozen settings, CAS/activation, owned imports/idempotency, PCM16 conversion/bounds, success/silence, complete Child observations, capacity reservation, saturation deadlines, cancellation/timeout, Future drop, retained-worker reset acknowledgement and quarantine accounting.

Intermediate failures were retained in saved logs and corrected: a missing runtime catalog accessor/import during implementation, the existing projection test's hard-coded 13-operation count, and the provider snapshot race found during Standards review. The final full suite passed after these fixes.

## Review

[Independent Standards and Spec reviews](review.md): no outstanding findings. The provider snapshot race was fixed and rechecked; existing TTS pin storage was left unchanged.

## Optional runtime evidence

| Evidence | Result |
|---|---|
| Real Zipformer inference through Core ASR | NOT_RUN |
| Real Gipformer inference through Core ASR | NOT_RUN |
| Simultaneous real Voice + Background contention | NOT_RUN |
| Cancellation during real noninterruptible offline decode | NOT_RUN |

Deterministic tests do not establish immediate physical cancellation. Native work without cleanup acknowledgement remains quarantined with its provider lease and capacity retained. Unicode-heavy or escaped transcripts may hit the bounded serialized ToolResult budget before the 16,000-character limit; overflow fails rather than truncating text.
