# ADR 0117 — Knowledge Core owns durable Wiki publication

Status: Superseded by [ADR 0127](0127-orchestrator-flow-task-and-wiki-runtime.md) (2026-10-22).

Agent Job V3 already has native Root/Sub-Agent Sessions, frozen Core Operations and Admin HTTP. A deployment-wide **Knowledge Core** owns Wiki Pages, immutable Sources, source provenance, revisions and publication. Admin handlers and future `core.knowledge` Job tool calls use **the same Rust Core interface**, never separate Markdown writers.

Core Plugin Policy governs new Agent tool starts, not ownership of Knowledge files or Admin permission. LLM output is an untrusted proposal. We reject fake External Plugin installations, MCP-as-local-storage, arbitrary file-write tools and a second orchestration runtime. Safe-read Job tools may ship before a separately authorized publication tool.

## Incremental implementation

This branch adds two built-in Core groups (disabled by default) with four safe-read operations, reusing existing frozen Job V3 choices and group policy. The read adapter must not gain any filesystem-write authority; Admin-only draft/fact mutations are separate interfaces. Existing Core groups remain enabled/unchanged.
