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

Job Core Plugin read-only integration is available as `core.knowledge` (`knowledge.search`, `knowledge.read`) and `core.memory` (`memory.recall`, `memory.summary`). Both groups start **disabled** by admin policy; enable explicitly after validation. Agent write/publish/forget is not exposed through this adapter.

## Branch implementation status (2026-10-16)

- Implemented: two distinct Rust Core modules, common canonical Markdown vault with CAS/revisions, process-wide lock, two SQLx/FTS5 derived indexes (with explicit forced rebuild), Admin endpoints and separate Vue workspaces.
- Implemented: immutable UTF-8 Markdown Source v1 import, SHA256/provenance verification, source-verified draft publication, Admin draft visibility and Agent-only verified Wiki reads/search.
- Implemented: two additional Core Job V3 groups disabled by default, with `knowledge.search`, `knowledge.read`, `memory.recall`, `memory.summary` read-only operations.
- Implemented: owner-scoped confirmed Memory Facts, correction/forget via explicit Admin interface and automatically generated `MEMORY.md`.
- Not implemented: full PDF/DOCX/photo/video/audio source ingestion and extraction, Job-generated Wiki publication/approval, Agent-triggered Memory writes, semantic embeddings/reranking, multi-owner Memory, background filesystem watcher, Git-based sync and advanced graph/quality dashboards. No hidden scope expansion into Job V3 scheduler.
- Verification: GitHub Actions is the executable Rust/TypeScript validation source. See branch CI check before merging; no local Rust toolchain available in this workspace.
