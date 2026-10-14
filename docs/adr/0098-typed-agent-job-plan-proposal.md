# ADR 0098 — Typed Plan Proposal before Rust-owned Executable Plan

## Status
Accepted for implementation, 2026-10-13. Native provider JSON-Schema qualification remains gated.

## Scope since Agent Job V2

Plan Proposal/PlanCompiler/append-only DAG replanning apply only to legacy `dag_v1` Jobs. New hierarchical Jobs use per-turn Rust-validated decisions and dynamic persisted Tool Steps (ADR 0103); they do not reinterpret existing DAG plans.

## Context
ADR 0094 establishes a bounded DAG with frozen plugin authority. The first implementation asked an Orchestrator LLM to reproduce executable JSON Schemas. Model output frequently failed strict JSON/Serde/contract validation and surfaced only `invalid_plan`. Retries lacked field-level feedback.

## Decision
An Orchestrator produces a **Plan Proposal**, an untrusted declaration of intended plugin/LLM steps, dependency keys and arguments. The Rust-owned **PlanCompiler** materializes an **Executable Plan** by binding each plugin operation to its exact frozen input/output contract and LLM steps to one server-owned result schema. Existing DAG, authority, publication and immutable Step checks remain authoritative; no plugin code executes before validation and atomic publication.

**Plan Repair** is a bounded retry of a rejected Proposal with typed diagnostics and the previous proposal as ephemeral model context. It never expands frozen authority or replays a worker attempt. **Replan** is a separate, bounded append-only proposal after a quiescent execution checkpoint. Previously published Steps, results and contracts remain unchanged. This narrows the previous ADR 0094 wording allowing replacement of pending Steps.

Only metadata describing plan attempts, rejections, publication and step/Job checkpoints becomes durable Job Events; raw prompts and model/plugin outputs do not.

## Alternatives considered
- Trust model-authored executable JSON Schema: rejected, brittle and transfers contract ownership to model output.
- Embed an external agent framework: rejected, duplicates the Rust scheduler, authority checks and SQLite checkpoints.
- Force function calling for every provider: rejected for now, because not all supported transports have qualified structured-tool behavior.

## Consequences
Plan Proposal JSON is not a stable public Job API. Existing persisted Steps remain valid. Providers without qualified native structured output use strict prompt JSON and repair; no silent provider-mode downgrade. Planner repair and accepted replan have separate limits. Administrator observes safe failures through the existing Jobs page and cursor-based events.
