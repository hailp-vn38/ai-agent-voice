# Ponytail review — applied simplifications

Accepted architecture remains three SQLite files, separate Knowledge/Memory ownership, Markdown as canonical, one internal Markdown/indexer implementation, existing Job V3 execution and Rust trusted writer.

1. Remove duplicated `kb_content`/`mem_content`; FTS5 holds search text; read full originals from Markdown.
2. One initial migration per new search database, rather than one per table.
3. Defer links, citation projection and relations tables until an actual query requires them; do **not** remove citation validation.
4. Reuse Agent Job V3 for ingest execution; no `knowledge_ingest_runs` state machine.
5. Facts are canonical; `MEMORY.md` is a regenerated view.
6. No separate `knowledge/log.md`; use revisions and Job Events.
7. Keep initial Admin Web Pages/Sources/Facts/Summary, defer speculative Activity/Quality/Review interfaces.
8. All DB/migration/recovery rules normatively defined in `database-layout-v3.md`; implementation/flow link to it instead of repeating SQL contracts.

Review is overengineering-only. None of these changes grants tool write permissions or weakens provenance, owner scope, consistency and recovery checks.
