# ADR 0121 — QMD-inspired Markdown passages inside the Rust Knowledge index

- Status: Accepted
- Date: 2026-10-16
- Scope: Knowledge/Memory V3 and Agent Job V3 read-only Core operations
- References: [QMD analysis](../knowledge/qmd-markdown-implementation-review.md), [Knowledge V3 flow](../flows/13-knowledge-v3.md), [ADR 0120](0120-three-sqlite-database-lifecycles.md)

## Context

Knowledge V3 stores authoritative Wiki and Memory content in bounded Markdown files, with independent FTS5 projections in `knowledge_index.db` and `memory_index.db`. The initial index treated an entire query as one exact FTS5 phrase and returned the first 600 characters of a document, rather than the most relevant passage. The larger the Wiki, the less useful that result is for an Agent. `tobi/qmd` solves comparable indexing and retrieval problems but also provides a JavaScript runtime, document-content database, embedding/LLM pipeline and CLI; adopting those would duplicate established Rust modules.

## Decision

1. **Preserve the external Seam.** `KnowledgeCore.search(query, limit)` and `KnowledgeCore.read_verified(id, from_line, max_lines)` remain the interface used by Job V3. The `SearchHit` representation is extended with *optional* `start_line`, `end_line` and `heading_path`; `source_ref` is retained. Existing frozen Core operation names and revisions do not change.
2. **Place query syntax inside SearchIndex.** A small `knowledge_lex` module converts bounded Unicode user/model text to escaped FTS5 phrases; it never executes client-controlled `MATCH` expressions. Strict conjunctive matching is followed by a bounded disjunctive fallback only if strict yields no authorized result. BM25 weights title more heavily than body. Both Knowledge and Memory reuse the lexical compiler, but their retrieval policies remain independent.
3. **Place Markdown analysis in a pure internal module.** `markdown_chunker::split_markdown(body)` computes heading-aware, UTF-8-safe passages with byte ranges, line numbers, ordinal, SHA-256, and an overlap. Fenced code is kept together (very long fences may form an oversize passage). No new public Core method or independent runtime is introduced.
4. **Derive passages in Knowledge's existing SQLite index.** SQLx migration `migrations/knowledge/0002_markdown_passages.sql` adds `kb_chunks` and `kb_chunks_fts` to **knowledge_index.db**, not to the application or Memory database. Each accepted verified canonical Wiki revision atomically replaces its previous chunk rows, chunk FTS, and existing whole-document row in one transaction. Force rebuild starts from canonical Markdown. Unpublished Wiki drafts do not appear in Agent chunk index.
5. **Query the best passage per Wiki.** FTS5 selects matching chunks and BM25 ranks them; SQL chooses one best passage per stable Wiki ID before limiting results, without truncating matching candidates prematurely. Search results include the body-relative line interval and heading path; the Agent can retrieve exact content through the established verified line-read method.
6. **Do not relax eligibility or provenance.** Knowledge Core validates the canonical Wiki and immutable Source reference when exposing Agent results; Memory recall is restricted to confirmed active owner facts. A missing root, incomplete scan or Source integrity problem retains fail-closed semantics.
7. **Do not introduce embedding yet.** Keyword retrieval is the baseline. Embeddings, sqlite-vec, RRF, query expansion and reranking require latency/recall evidence and a separate decision; they are not part of this migration.

## Interface and implementation rationale

- **Depth:** one read interface now hides safe lexical compilation, SQLite FTS, Markdown parsing, passage ranking and provenance. Job and Admin adapters do not learn chunking details.
- **Locality:** SQL stays under `src/database/knowledge_index.rs`; the pure Markdown parser remains under `src/services/knowledge/markdown_chunker.rs`. Tests exercise public Core reads/search plus pure parser logic.
- **Deletion test:** removing the parser would force line/range handling into the index writer or callers; removing the lexical compiler would distribute FTS5 escaping. Both earn a private seam; no hypothetical adapter is added.

## Migration and recovery

Knowledge indexer version advances to 2; Memory stays at version 1. On first open, SQLx creates the derived chunk tables, and the existing hash/revision/version reconciliation rebuilds accepted passages. Existing stable IDs, Wiki Markdown, published Source refs, Memory facts and Job history do not change. Index writes are transactional; failure leaves the health marker stale. A forced rebuild recreates all derived rows, including passage FTS. Filesystem and SQLite are not a distributed transaction; Markdown remains authoritative.

## Tests and deferred questions

Tests cover Vietnamese tokenization, compound identifiers, FTS control characters, published-vs-draft isolation, confirmed Memory gating, UTF-8/emoji/CRLF passages, fenced code, distant search hits, line spans, Source refs, restart and rebuild. Post-merge query-quality evaluation should compare FTS baseline and title-weighted passage retrieval on an actual Vietnamese corpus. A separate ADR is required if vector search becomes necessary.

## Conservative evidence-line qualification (2026-10-18)

FTS matches across passage title, heading and body are differentiated in the read projection. A result gains `match_field=body` and body-relative `start_line/end_line` only when all effective query matches arise from passage body. Title-only, heading-only or cross-field matches retain an excerpt and provenance but omit potentially misleading body-line citations. The optional `match_field` property extends the existing operation output without altering frozen choice identity. This keeps the earlier query/index design and CAS publication semantics intact.
