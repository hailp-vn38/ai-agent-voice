# ADR 0096 — Two global Background LLM Providers with Voice capacity reservation

## Status
Accepted architecture, 2026-10-13. Implementation is phased; acceptance is not execution readiness.

## Decision
Deployment-wide Admin `/background` has a revisioned enabled switch and two source-qualified Provider refs: Orchestrator LLM A and Sub-Agent LLM B. No per-Agent Template/Provider setting or plugin-provided model; both roles may share a provider only when non-Voice capacity is available. Runtime integrates existing Provider Catalog/secret loading, but prepares only LLM resources, never `SessionActor`/ASR/VAD/TTS. Voice-reserved capacity, including single-slot Voice/Diagnostic exceptions, is not treated as Background capacity.

## Why / consequences
One configuration plane avoids provider sprawl and fake Voice Session costs. A Job pins refs, resolved revisions/digests and immutable limits at submit; later setting changes cannot hot-swap it. Structurally unavailable capacity returns a typed preflight error, temporarily busy eligible capacity may wait only to Job deadline. This complements existing ADR 0080 and 0086. Web does not edit TOML runtime quotas.

## Integration clarification v1.5.1
Provider admission, installer builds, Python container capacity and Voice/ASR/TTS reservations have separate bounded permits. Deployment acceptance measures Voice responsiveness under simultaneous Reminder + Background + installer load rather than assuming two worker containers are harmless.

## P0 implementation boundary
P0 implements revisioned settings, exact source-qualified provider snapshots, and Background capacity admission. Planner/Worker LLM execution and runtime leases remain P3.
