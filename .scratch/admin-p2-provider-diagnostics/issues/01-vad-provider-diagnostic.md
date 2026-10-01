# 01: VAD Provider diagnostic

Type: task

Status: resolved

**What to build:** Expose a bounded VAD readiness/inference diagnostic for an already-loaded
database Provider through the Admin API.

## Acceptance criteria

- [x] `POST /api/admin/providers/{key}/test/vad` uses no caller PCM and runs one canonical silent
  frame through only the already-loaded VAD runtime.
- [x] The route validates desired row/type/enabled/runtime state and retains Voice-reserved worker
  capacity and diagnostic timeout/quarantine semantics.
- [x] The success response is bounded, secret-free and reports provider key, type, probability,
  sample range, elapsed time and runtime provenance.
- [x] Router-seam regression tests cover success and an incompatible Provider key.

## Comments

- 2026-10-01: Vision remains deferred because the source assessment explicitly requires its
  ProviderType/adapter/database/materialization/binding contract first.
- 2026-10-01: Validated by `cargo fmt --check`, `git diff --check`, focused Admin test and
  `cargo test --workspace --quiet -- --test-threads=1`.
