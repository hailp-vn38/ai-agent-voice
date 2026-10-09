# ADR 0084 — Compose one structured System prompt per conversational turn

## Status

Accepted and implemented, 2026-10-09.

## Decision

Deployment prompts are rendered once into the immutable session-profile base prompt with the strict `agent_name`, `persona`, `language`, and optional `speakers_info` placeholders. At speech delivery, the actor composes exactly one System message from that snapshot and the current turn's verified Speaker Context; tool continuation reuses that same message. Stored Agent Template prompts remain already-rendered content: an exact optional `{{speakers_info}}` slot is substituted per turn, while a legacy prompt without it gets an appended block only for a verified match.

Speaker data is bounded JSON in a `<speakers_info>` data block, filters control characters, escapes XML delimiters, and explicitly says that a verified voice match is not authorization. It is neither persisted in Dialogue History nor carried into a later turn. The composed System message must remain within the existing 96 KiB limit before the LLM request is made.

## Consequences

Prompt templates stay intentionally single-pass rather than becoming a general scripting language. A missing, unknown, timed-out, or unverified speaker never inherits a prior identity; it either replaces the optional slot with an explicit unknown marker or leaves a legacy prompt unchanged.
