# ADR 0120 — Keep application DB and two independent search indexes

Status: Superseded by [ADR 0127](0127-orchestrator-flow-task-and-wiki-runtime.md) (2026-10-22).

The existing `database.url` SQLite/SQLx schema remains the canonical operational database for Agents, Plugins, Jobs and Events. Add only `knowledge_index.db` and `memory_index.db` as independently migrated **derived** FTS5 projections of canonical Markdown. The two new files have separate migration histories/pools; never run the application's migrator on them or add cross-database foreign keys/transactions. Each may be rebuilt without affecting the other or the existing Job runtime.

This adds modest operational isolation and prevents personal Memory queries from mixing with general Wiki search; in exchange, filesystem/SQLite coherence requires explicit stale detection, idempotent reconciliation, and backup of the canonical files separately from the main DB.
