# ADR 0012 — Quick Speaker Enrollment uses provisional voiceprints

## Status

Accepted, 2026-10-08.

## Decision

The Admin API stages one normalized, bounded speaker embedding for ten minutes, then atomically
creates a Speaker and a `pending` voiceprint only when the operator explicitly commits it. The
staged waveform is never stored. A committed capture retains only a 24-hour non-biometric
idempotency tombstone.

`browser_validation_status='pending'` is the sole assurance state for Quick Enrollment. It can be
used by explicitly granted Observe diagnostics but is rejected by server-side `required` policy
qualification and session admission. Full Enrollment remains the only path that publishes
`passed`, and calibration still requires its existing exact-candidate-set evidence.

## Consequences

Quick registration is shorter without claiming identity verification. Capture and commit are
separate authenticated operations, provider/runtime provenance is rechecked at commit, and a
retry with the same capture cannot create another Speaker. Candidate-set digests include
validation status so promotion or downgrade invalidates prior qualification evidence.
