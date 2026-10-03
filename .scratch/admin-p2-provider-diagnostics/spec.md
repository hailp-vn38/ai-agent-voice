# Admin P2 provider diagnostics

## Goal

Add the VAD Provider Card diagnostic at the existing authenticated Admin boundary without
materializing a provider, reading a secret, or changing a Voice Session.

## Scoped route

- `POST /api/admin/providers/{key}/test/vad` has no request body. It runs exactly one canonical
  512-sample, 16 kHz silence frame against the already-loaded VAD runtime and returns bounded
  probability/range metrics plus runtime provenance.

## Deferred work

- Vision diagnostic remains deferred: Vision is not a database ProviderType or Template binding,
  so `test/vision` has no approved desired-state/runtime contract.
- Resource deletion policy remains deferred by the source assessment.

## Public seam

Authenticated `/api/admin` HTTP routes in `crates/voice-agent-server/tests/admin_api.rs`.
