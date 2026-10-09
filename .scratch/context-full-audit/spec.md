# CONTEXT.md full audit

## Goal

Reconcile the repository's domain vocabulary with the current implementation and accepted ADRs, updating only `CONTEXT.md` where terminology, lifecycle, ownership, or bounds differ.

## Audit groups

1. Protocol, audio, voice-session state, worker ownership, dialogue and delivery.
2. Provider configuration, assets, runtime catalog/manager, benchmarks and qualifications.
3. SQLite desired configuration, admission, enrollment, lifecycle, secrets, Admin API and external MCP.
4. Speaker recognition, calibration, policy, grants and tool allowlists.
5. Admin Web public read model and current API terminology.
6. Cross-cutting prompt/template semantics and each accepted ADR not already covered above.

## Method

For each group, compare every relevant `CONTEXT.md` term against its owning Rust/Vue module and accepted ADR. Preserve existing user changes. Add a term only when code has a stable domain boundary missing from the context; remove or revise wording only when current code contradicts it. Validate Markdown/diff hygiene after the final pass.

## Scope

Documentation only: no production code, schemas, API behavior, tests, fixtures, or configuration changes.

## Audit result

Completed against the current Rust server, Admin Web, flow documents, and accepted ADRs. `CONTEXT.md` now records the prompt/speaker turn boundary, SessionActor and single-writer ownership, protocol-fault handling, Admin-managed resource credentials, Speaker Voiceprint, and current Quick Speaker Enrollment behavior.

### ADR discrepancy

ADR-0012 says a Quick Enrollment commit creates a `pending` voiceprint, while the current capture transactions write `browser_validation_status='passed'`. The context records the implemented behavior; this ADR should be reconciled separately before relying on it as the authority for enrollment assurance.
