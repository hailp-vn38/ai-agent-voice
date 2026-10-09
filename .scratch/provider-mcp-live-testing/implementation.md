# Provider/MCP Live Testing implementation

Implemented the seven draft/saved diagnostic routes, source-aware managed Provider leases, request-local credentials, RMCP connection/full discovery, Provider create/detail/edit testing, and MCP detail/catalog/form testing. API contracts are documented in [live-testing.md](../../docs/api/live-testing.md) and Postman. Domain terminology is recorded in `CONTEXT.md`.

## Source reconciliation

The guide started at `37641bc`. During implementation, `2769d0a` committed the existing UI header/card changes, including the initial MCP title link. The final diff builds on that commit, preserves its layouts/actions, and expands only the MCP card body link. No rewrite of unrelated pages or user changes was needed. The now-tracked DetailHeader test had two invalid Vue Test Utils `.get().exists()` assertions; these were changed to `.find().exists()` to restore the full web typecheck. TypeScript 5.9 requires `ignoreDeprecations: "5.0"`, matching the locked dependency.

## Verification

- Default server suite: 594 passed, 4 ignored, zero failures across 52 test targets including doc tests.
- Qualification Provider HTTP gate: 1 passed; actual deterministic LLM/ASR/TTS inference through the production FactoryMaterializer and managed runtime; no persistence.
- Web: 36 Vitest files / 119 tests passed; full typecheck and production build passed.
- `cargo fmt --all -- --check` and `git diff --check`.
- Explicit regression gates: bounded multipart and zero-Hz WAV rejection; distinct draft logical identities and resource budgets; cancellation ownership until terminal ACK; MCP JSON/SSE pagination, empty catalogs, invalid later page, encrypted saved credentials/revision compatibility, no writes/tools calls, and cancelled-waiter admission/shutdown.

The full qualification-feature server suite has pre-existing non-exhaustive fixture destructuring for `VadInstanceConfig` in unrelated integration tests (`transcript_archive`, `session_profile`, `external_mcp_admission`, `tool_round_executor`, `protocol_e2e`). The targeted qualification gate above and full default suite are used here; those fixture migrations are outside this implementation.

Not checked: physical microphone/speakers, real external credentials, and browser visual smoke for mobile/light/dark. Deterministic tests cover WAV encoding/resampling, paused audio exclusion, track cleanup, late permission grants, TTS URL lifetime and metadata, navigation/action separation, escaped catalog data, and no automatic probes.

## Standards

Review against `2769d0a`: no mandatory violations. One non-blocking Duplicated Code heuristic: the bounded memory-pressure eviction retry appears in saved and draft acquire methods. Kept the two small source-specific entry paths; both call the same private acquisition and eviction logic. No extra abstraction was added for this review suggestion.

## Spec

Initial review found four issues: local Provider edits incorrectly adopted saved credentials; changed prompt left prior success visible; invalid MCP Header auth fell back to saved Bearer; modality UI lacked requested controls/metrics. All four were fixed and regression-tested. Follow-up review found no remaining bug in the corrected paths.

Review counts: Standards 0 mandatory + 1 heuristic; Spec 4 found, 4 fixed, 0 outstanding.
