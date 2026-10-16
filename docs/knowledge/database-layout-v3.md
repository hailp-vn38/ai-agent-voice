# Three SQLite lifecycles — normative storage specification

## Ownership

| Database | Canonical authority | Content | Migrator |
|---|---|---|---|
| Existing application `database.url` | SQLite | existing Agents, Providers, Plugins, Jobs, Job Events | existing `crates/voice-agent-server/migrations` |
| `<main database directory>/knowledge/knowledge_index.db` | Wiki Markdown and immutable Sources | `kb_documents`, `kb_fts`, `kb_index_state` | `migrations/knowledge` |
| `<main database directory>/memory/memory_index.db` | Memory Fact Markdown | `mem_documents`, `mem_fts`, `mem_index_state` | `migrations/memory` |

Use **one initial SQL migration per index database**, not one file per table. Later migrations only when needed; the existing main SQLx migrator is never applied to either index. WAL, busy timeout, foreign keys and schema version checks belong to each pool. Main database path and migration history must not change.

## Lean indexes

The search files are **rebuildable**. `kb_documents` tracks stable ID, kind, title, relative path, revision, SHA256 and scanner fingerprint. `kb_fts` contains the only SQLite search-text copy. `mem_documents` additionally records `owner_scope`, `status` and `confirmed`; every memory result filters by all three. Both track independent health. No `kb_content`, `mem_content`, `kb_links`, `kb_citations`, `mem_relations` or second ingest-run ledger in MVP.

Never derive persistent identity from the first six characters of SHA256; content changes. No cross-index SQL JOIN or cross-file atomic transaction. SQLite search rows are never the source of accepted knowledge or user facts.

## Incremental scanner

- Enumerate only canonical `.md` files within an allowed root; never follow symlinks outside that root.
- Current scanner computes SHA256 and parses bounded frontmatter on every complete vault scan; unchanged path/hash/revision/indexer-version rows skip a SQL rewrite. Timestamp and size are stored for diagnostics, not trusted as a sole hash-skipping cache. Add an optimized fingerprint fast path only after its correctness is tested.
- Index both metadata and FTS body in **one local DB transaction**. Use stable ID for rename/move, reject duplicated IDs, and remove stale rows only at the end of a fully successful scan.
- Missing root, parse error, I/O error or interrupted scan must preserve indexed rows and mark unhealthy; do not represent an unavailable mount as an empty collection.
- `indexer_version` bump forces reprocessing. Any index-DB corruption can be repaired by discarding **only that index** and rebuilding canonical files; never destroy files as part of rebuild.
- FTS5/BM25 with bounded line reads in MVP; chunk vectors and rerank are deferred. Owner scope must be trusted request context, not a model-selected string.

## Crash and backup

Canonical atomic Markdown rename may succeed before FTS update: publication stands, search becomes stale until reconciliation. `MEMORY.md` is generated from confirmed active facts and checked against a freshly regenerated expected summary on each read; old summary is withheld on failure. A separate persistent generation marker is not implemented. Existing application DB backup uses SQLite Online Backup API/coordinated snapshot (not a live `.db` file-only copy under WAL). Vault originals, Markdown and revision files must be backed up consistently; the two indexes may be regenerated. Git does not back up binary originals.

## SQL migrations

- [Knowledge index initialization](../../crates/voice-agent-server/migrations/knowledge/0001_index.sql)
- [Memory index initialization](../../crates/voice-agent-server/migrations/memory/0001_index.sql)

Avoid independent `knowledge_ingest_runs` status transitions. Job V3 alone owns run/retry/cancel outcomes; published Wiki and immutable Source version own canonical publication/provenance.

## Explicit rebuild versus incremental reconciliation

Ordinary managed writes reconcile changed canonical records by stable ID/content hash. Admin `index/rebuild` performs a **forced FTS rebuild** in one SQLite transaction, including orphan cleanup and repair of missing FTS matches even when Markdown hashes are unchanged. It never deletes canonical Markdown. A missing root or partial scan prevents commit and leaves search health stale. This is intentional, rather than assuming a hash-equal document guarantees its derived FTS row is intact.

## Runtime path and locking

The implementation locates Knowledge/Memory vaults in sibling directories of the existing durable application SQLite file (`Database::durable_path().parent()`). The application-owned Runtime is shared by Admin and Agent Job; its process-wide async vault lock serializes canonical writes, search and reconciliation, so they do not perform competing writes/index snapshots inside one Rust server. The lock **does not** coordinate two different server processes. A second process pointed at the same vault must be disallowed until an explicit cross-process lock protocol exists. This branch does not add a separately configurable knowledge root.

## Revisions and derived search integrity

The filesystem now distinguishes accepted Wiki `wiki/<id>.md` from unaccepted replacement `drafts/<id>.md`. A staged record stores `draft_base_revision`, and the staged CAS is validated independently; only the accepted canonical revision is indexed. `draft_revisions/` archives drafts without polluting canonical `revisions/`. No fourth SQLite database is needed. Provenance validation at publish alone is insufficient: Knowledge Core also verifies the original Source checksum when serving verified Agent results and on startup/rebuild; failures mark the Knowledge index stale, not Memory.

The editor uses complete, size-bounded Markdown reads. Limited line-range reads are never fed into a full-document save operation. A canonical vault that existed in the previous process carries `.vault_initialized` and must fail closed on restart if its document folder is missing. A separate `.drafts_initialized` protects the staged drafts directory. Crash-safe immutable history uses temporary-file fsync plus atomic rename and directory fsync. These markers are not an authorization or backup mechanism.

An outer pair of initialization markers in the main database directory (`.knowledge_v3_initialized` and `.memory_v3_initialized`) additionally protects against **whole vault** removal, where an in-vault marker would disappear with the data. Initialization writes these markers only after a successful open/reconciliation. This does not require a fourth database or cross-index transaction.

## Passage index migration (Knowledge-only)

`knowledge/0002_markdown_passages.sql` adds `kb_chunks` (stable Wiki ID, accepted revision, ordinal, heading path, byte/line ranges, content hash and passage body) and `kb_chunks_fts` (FTS5 for title, heading and passage text). These remain *derived* rows and live in `knowledge_index.db`, not the main app or `memory_index.db`. The Knowledge indexer version increments to 2; the Memory indexer stays 1.

A complete canonical scan plus SQL transaction replaces each changed Wiki's old passage/FTS rows and commits `ready` together with the new revision. Force rebuild first clears derived passage tables inside that same transaction. Deletion on successful complete scan retires both documents and their passages. Rollback keeps the prior projection and the search health marker stale. Staged Wiki draft files are excluded from the Agent chunk index; Memory facts are intentionally not chunked.
