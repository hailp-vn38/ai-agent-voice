# ADR 0117 — Knowledge Core owns durable Wiki publication

Status: Accepted for Knowledge V3 implementation.

Agent Job V3 already has native Root/Sub-Agent Sessions, frozen Core Operations and Admin HTTP. A deployment-wide **Knowledge Core** owns Wiki Pages, immutable Sources, source provenance, revisions and publication. Admin handlers and future `core.knowledge` Job tool calls use **the same Rust Core interface**, never separate Markdown writers.

Core Plugin Policy governs new Agent tool starts, not ownership of Knowledge files or Admin permission. LLM output is an untrusted proposal. We reject fake External Plugin installations, MCP-as-local-storage, arbitrary file-write tools and a second orchestration runtime. Safe-read Job tools may ship before a separately authorized publication tool.
