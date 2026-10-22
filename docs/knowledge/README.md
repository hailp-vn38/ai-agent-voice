# Knowledge + Memory V3

Status: implementation branch. Base: `feat/agent-job-v3-opencode-session-runtime`.

Architecture is **file-first hybrid**. Knowledge and Memory are separate domain modules with one internal Markdown/indexing implementation. The existing application SQLite DB remains authoritative for Agent Jobs, Providers, Plugins, and all execution history; `knowledge_index.db` and `memory_index.db` are independently rebuildable read projections.

- [Implementation and staged acceptance](implementation-v3.md)
- [Knowledge fixed Job: skill/prompt riêng, tool loop và tự publish — phân tích yêu cầu, chưa triển khai](ai-ingest-fixed-job-requirements-v3.md)
- [Knowledge fixed Job — module/interface, data, recovery và thứ tự triển khai](ai-ingest-fixed-job-implementation-v3.md)
- [ADR 0125 — native Knowledge tools, Job Skills và tự publish](../adr/0125-knowledge-job-tools-skills-and-publication.md)
- [AI Ingest, Review Queue, Auto Publish and Knowledge Provider Settings (proposed; aligned to current branch)](ai-ingest-auto-publish-implementation-v3.md)
- [2026-10-19 branch delta audit: 145 commits and implementation seams](ai-ingest-branch-reconciliation-2026-10-19.md)
- [ADR 0123: Knowledge execution and publication ownership](../adr/0123-knowledge-ingest-job-execution-and-review-policy.md)
- [Database schema, migrations, recovery and backup](database-layout-v3.md)
- [Web UI scope](web-ui-v3.md)
- [Flow](../flows/13-knowledge-v3.md)
- [Wiki Graph View / Obsidian interoperability — implementation plan](wiki-graph-view-implementation-v3.md)
- [ADR 0124 — canonical Wikilinks and derived Graph projection](../adr/0124-wiki-wikilinks-graph-projection.md)
- [Knowledge Graph target flow](../flows/14-knowledge-graph-v3.md)
- [Domain vocabulary](glossary-additions.md)
- [ADRs 0117–0120](../adr/0117-knowledge-core-plugin-adapter.md)
- [Ponytail simplifications](ponytail-remediation.md)
- [Verification and remaining gates](verification-v3.md)
- [ADR 0121: QMD-style Markdown passages](../adr/0121-qmd-inspired-markdown-passages.md)
- [QMD Markdown indexing and retrieval comparison](qmd-markdown-implementation-review.md)

Reference patterns: Javis OS (distinct Wiki/Memory), QMD (incremental filesystem scan, FTS5/BM25, metadata and range reads). We do **not** embed QMD's JavaScript runtime and do not treat QMD's content-hash docid as a stable page ID.

This branch implements the Rust Knowledge/Memory foundation, read-only Core Job V3 tool integration, a Markdown-only Source import/publish path, separate Vue Admin workspaces, and an Obsidian-style Wiki Graph View (CI and manual Obsidian acceptance pending). Binary ingest, automated Agent writing, Voice prompt wiring, embeddings/reranking and advanced graph remain separate acceptance items; do not mark them shipped on the strength of these migrations.

Job Core Plugin read-only integration is available as `core.knowledge` (`knowledge.search`, `knowledge.read`) and `core.memory` (`memory.recall`, `memory.summary`). Both groups start **disabled** by admin policy; enable explicitly after validation. Agent write/publish/forget is not exposed through this adapter.

## Branch implementation status (Knowledge V3 baseline, updated 2026-10-19)

- Implemented: two distinct Rust Core modules, common canonical Markdown vault with CAS/revisions, process-wide lock, two SQLx/FTS5 derived indexes (with explicit forced rebuild), Admin endpoints and separate Vue workspaces.
- Implemented: immutable UTF-8 Markdown Source v1 import, SHA256/provenance verification, source-verified draft publication, Admin draft visibility and Agent-only verified Wiki reads/search.
- Implemented: two additional Core Job V3 groups disabled by default, with `knowledge.search`, `knowledge.read`, `memory.recall`, `memory.summary` read-only operations.
- Implemented: owner-scoped confirmed Memory Facts, correction/forget via explicit Admin interface, an Admin-only overview including `needs_review`/`forgotten`, and automatically generated `MEMORY.md`. Forget is **logical suppression**, not physical erasure of historical revisions or backups.
- Not implemented: full PDF/DOCX/photo/video/audio source ingestion and extraction, Job-generated Wiki publication/approval, Agent-triggered Memory writes, semantic embeddings/reranking, multi-owner Memory, background filesystem watcher, Git-based sync and advanced graph analytics/quality dashboards. No hidden scope expansion into Job V3 scheduler.
- Verification: GitHub Actions is the executable Rust/TypeScript validation source. See branch CI check before merging; no local Rust toolchain available in this workspace.

Remediation after the two-axis code review: accepted Wiki revision editing uses a distinct staged Markdown folder and no new database; canonical roots are protected against restart-time mass-pruning, and Agent read results carry source provenance. The Core implementations and tests now live in separate modules/files, preserving existing public interface paths.

## New Knowledge Graph and Settings surfaces (2026-10-19)

The branch now includes a canonical-Markdown Wiki link resolver (`services/knowledge/wiki_links.rs`), Knowledge-only migration `0003_wikilink_graph.sql` with rebuildable `kb_links`, Knowledge Index version **3**, a typed Admin graph route, global/local Cytoscape Graph View, backlinks, outgoing/unresolved link diagnostics and link-aware Wiki move protection. The physical Raw/Wiki/Drafts explorer remains authoritative. The Web offers Reading/Compose/Graph modes and Obsidian-inspired note creation. Obsidian interoperability still needs the separate native smoke test.

A **dedicated `/knowledge/settings` route** exists, linked by the vault-footer gear, but its `KnowledgeSettingsPage.vue` is **example-only**: no persisted Knowledge provider settings or review controls. AI ingest, `knowledge_ingest_v3`, auto-publish and Review Queue are not implemented. Implement new settings **on that existing page**, not inside the physical Raw/Wiki/Drafts selector. See the [branch audit](ai-ingest-branch-reconciliation-2026-10-19.md).

## QMD-inspired Markdown retrieval implementation

The Rust Knowledge index now has safe lexical FTS5 query compilation, title-weighted BM25, relevant passages and body-relative line citations. Verified canonical Wiki documents are split by headings/paragraphs with fenced-code awareness into `kb_chunks` / `kb_chunks_fts` in `knowledge_index.db` (migration `0002_markdown_passages.sql`, indexer version 2; subsequently version 3 for the graph projection). The memory index remains document-level (version 1). Job operation names, plugin registry, source verification and the three-database file-first lifecycle remain unchanged. See [ADR 0121](../adr/0121-qmd-inspired-markdown-passages.md). Embedding/hybrid search remains deferred.
