# Knowledge + Memory V3

Status: implementation branch. Base: `feat/agent-job-v3-opencode-session-runtime`.

Architecture is **file-first hybrid**. Knowledge and Memory are separate domain modules with one internal Markdown/indexing implementation. The existing application SQLite DB remains authoritative for Agent Jobs, Providers, Plugins, and all execution history; `knowledge_index.db` and `memory_index.db` are independently rebuildable read projections.

- [Implementation and staged acceptance](implementation-v3.md)
- [Database schema, migrations, recovery and backup](database-layout-v3.md)
- [Web UI scope](web-ui-v3.md)
- [Flow](../flows/13-knowledge-v3.md)
- [Domain vocabulary](glossary-additions.md)
- [ADRs 0117–0120](../adr/0117-knowledge-core-plugin-adapter.md)
- [Ponytail simplifications](ponytail-remediation.md)

Reference patterns: Javis OS (distinct Wiki/Memory), QMD (incremental filesystem scan, FTS5/BM25, metadata and range reads). We do **not** embed QMD's JavaScript runtime and do not treat QMD's content-hash docid as a stable page ID.

This branch introduces a bounded Rust core and Admin HTTP foundation first. Core Job tool integration, binary ingest, Vue pages, embeddings/reranking and advanced graph are separate acceptance items; do not mark them shipped on the strength of database migrations.
