# ADR 0082 — Unified Speaker Enrollment is the default; Quick remains provisional

## Status

Accepted, 2026-10-08.

## Decision

The Admin Web's default **Add Speaker** flow creates a profile, collects the server-configured
Full Enrollment samples, then collects an independent holdout. The browser calls the existing
draft, sample, validate, and finalize Admin API operations in order. It publishes a voiceprint
only after the server reports `passed` for the current revision and the existing finalize
operation succeeds.

Quick Enrollment remains an explicit alternate flow. It creates exactly one `pending`
voiceprint and never promotes that sample to `passed`. A pending Speaker can use the same Full
Enrollment wizard later, with a new sample set, without creating a second Speaker.

The wizard does not grant an Agent/Template binding, enable `required`, or create qualification
evidence. Those remain separate server-side policy and Required-qualified Calibration decisions.
No WAV, embedding, or score is persisted by the browser.

## Consequences

The client owns only UX orchestration and always uses server revisions returned by mutations.
Closing an in-progress wizard preserves the profile and draft for resumption; cancelling a draft
does not delete the Speaker profile. This supersedes the default UX in ADR 0012, while retaining
all of ADR 0012's Quick-assurance and idempotency guarantees.
