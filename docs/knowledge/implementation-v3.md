# Knowledge + Memory V3 — implementation contract

## Invariants

1. Distinct `KnowledgeCore` (Wiki Pages, Sources/provenance) and `MemoryCore` (confirmed owner facts, correction/forget). Neither domain implicitly promotes into the other.
2. Canonical Wiki/fact content is Markdown, with stable `id`, monotonic `revision` and an expected-revision compare-and-swap. Source originals/versions and accepted revisions must survive index loss.
3. Rust is the only trusted writer. A proposed change from LLM/Root/Child remains data, not a direct filesystem operation.
4. Application DB configured by `database.url` is unchanged; index DBs are derived, independently migrated and may lag. Search refuses to call stale results authoritative. No cross-file SQL transaction or foreign keys.
5. Memory facts are canonical; bounded `MEMORY.md` is a regenerated read view, **not** a second source of truth. A forgotten fact must be suppressed from live recall and summary before success is acknowledged.
6. Reuse Agent Job V3 for execution state; do not implement a second ingest state machine. Plugin group enable/disable must not delete canonical data.

## Deep modules

| Module | Interface | Implementation |
|---|---|---|
| KnowledgeCore | publish/read/search/rebuild | canonical accepted Wiki vault, separate staged draft vault, Source verification, revision history and Knowledge search index |
| MemoryCore | confirm/correct/forget/recall/summary/rebuild | owner-scoped canonical facts, generated summary and Memory search index |
| MarkdownVault (internal) | read/write(expected_revision)/scan | frontmatter validation, stable identity, lock, snapshot, atomic replacement |
| SearchIndex (internal) | rebuild/search/health | domain-specific SQL schema, filesystem/hash reconciliation and FTS5 |
| Admin adapter | authenticated Knowledge/Memory endpoints | delegates to the cores; no independent SQL/FS mutations |
| Job Core Plugin read adapter | frozen typed safe-read operations (disabled by default) | delegates to same cores; no new runtime or tool registry |

## On-disk shape

```text
data/
  <main SQLite database path selected by database.url>
  .knowledge_v3_initialized             # outside Knowledge vault
  .memory_v3_initialized                # outside Memory vault
  knowledge/
    wiki/<stable-id>.md                 # last accepted revision
    drafts/<stable-id>.md               # staged edit of verified Wiki
    draft_revisions/<stable-id>/<revision>.md
    .vault_initialized                  # restart root-loss guard
    .drafts_initialized                 # staged root-loss guard
    raw/<source-id>/<version>/original.<ext>   # immutable
    raw/<source-id>/<version>/extracted.md
    revisions/<stable-id>/<revision>.md
    knowledge_index.db
  memory/
    facts/<stable-id>.md
    revisions/<stable-id>/<revision>.md
    MEMORY.md                                  # derived
    memory_index.db
```

Frontmatter examples:

```yaml
---
id: wiki_01
kind: wiki
title: Agent Session Runtime
revision: 1
status: verified
source_ref: src_01:1
---
Document body.
```

```yaml
---
id: mem_01
kind: memory
title: Preferred language
revision: 1
status: active
scope: owner
confirmed: true
source_ref: conversation:turn-ref
---
User prefers Vietnamese for voice responses.
```

Content hash lives in the derived index, never as a self-referential field inside Markdown. Stable IDs survive title and path changes. Unknown frontmatter must not be silently overwritten when editing imported documents. Citations reference immutable Source versions, not only a Job Artifact. The `knowledge.search` and `knowledge.read` Job contracts now include `source_ref`; `knowledge.read` also returns `from_line`. Search excerpts do not yet guarantee an exact matching line offset.

## Safe write and recovery sequence

Validate domain/ID/scope/source/size; serialize per-vault writes; verify `expected_revision`; snapshot old revision; write to same-directory temporary file, flush and rename atomically; reconcile domain index; regenerate bounded Memory summary when Memory changed. Filesystem + SQLite cannot be committed atomically: accepted Markdown remains authoritative if FTS update fails. On start or explicit rebuild, compare indexed hashes against canonical files, do not mass-delete after missing root, parse errors or incomplete scan. Index ready means the last full scan succeeded.

Never return a stale `MEMORY.md` after a forget; if summary regeneration fails, report failure and suppress old summary. Historical revisions and external backups have independent retention; do not promise physical erasure without a separate purge policy.

## Staged acceptance

- P0: distinct Rust cores, canonical markdown writes/revision CAS, independent SQLx migrations, FTS search/recall, rebuild and tests.
- P1: hardened original Source import and provenance, Admin UI Pages/Sources/Facts/Summary with index diagnostics.
- P2: read-only `core.knowledge` / `core.memory` frozen operations in current Job V3 catalog (implemented); Agent write proposal/approval gate, Voice summary wiring and Root/Child ingest (not implemented).
- P3: formats, optional FTS chunking, embeddings/reranking, optional Git sync, graph/quality flows.

Only label a stage complete when its acceptance is exercised, rather than inferring that migrations imply full product integration. See [database-layout-v3.md](database-layout-v3.md).

## Job V3 read-only adapter (branch implementation)

The Rust static Core catalog adds two disabled-by-default groups `core.knowledge` and `core.memory` via main DB migration `0122_core_knowledge_memory_policy.sql`. Four read-only tools (`knowledge.search`, `knowledge.read`, `memory.recall`, `memory.summary`) use frozen choices, inherited Step budgets and schema validation; operations are scoped by server, never model-selected file paths or owner scopes. The main Job DB records execution; no new ingest ledger. No write tools are exposed until provenance/approval and publication integration are implemented.

## Source-verified publication slice

Authenticated Admin can import bounded UTF-8 Markdown as immutable v1 Source (`raw/src_UUID/1/{original.md,extracted.md,manifest.json}`) and publish a draft only after Rust verifies `source_ref` (`src_UUID:1`), original SHA256, provenance and expected revision. Agent Job V3 `knowledge.search` and `knowledge.read` refuse drafts, while Admin can search/read drafts. PDF/image/video extraction, semantic chunking and Job-driven publication remain separate work.

## Admin-only Memory fact inventory

`GET /api/admin/memory/facts` without `q` lists bounded canonical fact metadata from Markdown, including `needs_review` and `forgotten`, so pending entries can be confirmed after a restart. With `q`, the endpoint uses the confirmed/active search projection. This Admin listing is **not** a Core Job operation. Forgotten entries are omitted from `memory.recall` and the derived `MEMORY.md`, and cannot be reactivated by editing the same ID. A new fact requires a new ID and explicit confirmation. All historical revision files remain retained unless an independent purge policy is implemented.

## Admin browsing versus Agent retrieval

An authenticated Admin can list bounded canonical Wiki metadata without a search term (`GET /api/admin/knowledge/pages?limit=100`) and similarly list all owner Fact metadata (`GET /api/admin/memory/facts?limit=100`). Both return file-backed descriptions, including drafts and unconfirmed records, for management only. Agent Job's `knowledge.search`/`knowledge.read` are restricted to `verified` Wiki; `memory.recall`/`memory.summary` are restricted to confirmed active owner facts. These admin-only list interfaces are deliberately absent from the Core Job operation catalog.

## Remediation — staged Wiki edits, restart and provenance

- Edits of an accepted `verified` Wiki Page are stored under `knowledge/drafts/`, with `draft_base_revision` referencing the canonical base. The canonical `knowledge/wiki/` page and Agent FTS index remain at the last accepted revision until Admin explicitly publishes a Source-verified replacement. Draft history uses `draft_revisions/`, distinct from canonical `revisions/`; no additional SQLite DB is introduced. Admin full-page reads prefer a valid staged draft; Agent operations always read only the canonical verified page.
- Publishing requires the staged revision CAS and unchanged canonical base; accepted canonical revision advances once, then the staged file is removed. If a crash occurs after commit but before deletion, a matching stale staged copy is ignored and may be cleaned before the next draft. A mismatched stale draft fails closed.
- A persisted vault-initialization marker prevents automatic recreation of a missing canonical/draft folder on restart. Legacy vaults with existing indexes are also detected. Indexes are not pruned when the root vanishes. Revision archives now write temporary files, fsync, rename, and fsync their parent directory before canonical publication.
- Admin editor uses a separate complete (size-bounded) Markdown read; line-range reads are only for browsing and Agent tools. Source SHA-256 is verified at publish, Agent read/search and Knowledge rebuild/recovery; missing/tampered verified evidence marks only the Knowledge index stale, leaving Memory independently available.
- Admin and Background Job adapters share one application-owned `Arc<OnceCell<Arc<Runtime>>>`; no duplicate SQL index pools are constructed for the same server instance. Blocking directory fsync is offloaded from Tokio runtime workers.

These are corrections within the existing KnowledgeCore/MemoryCore interfaces and three-database decision, not a second publishing engine.

The top-level `.knowledge_v3_initialized` and `.memory_v3_initialized` guards are written in the durable application SQLite's parent directory after successful initialization. They detect the **entire** vault root disappearing, while the in-vault markers detect missing canonical collections or staged draft directories. Neither marker replaces backups. A vault missing at restart is rejected rather than silently re-created.

## QMD-inspired lexical retrieval and passage projection

P1/P2 are implemented in the existing Rust modules, without a QMD runtime:

- `database/knowledge_lex.rs`: a bounded and Unicode-normalized FTS5 query compiler converts ordinary terms and quoted/compound phrases into SQL-parameterized MATCH expressions. Conjunctive matching is preferred; a disjunctive fallback executes only if no permitted strict match exists. Title is weighted above body using BM25; excerpts are drawn from FTS5 snippets near the query, not the first 600 characters.
- `services/knowledge/markdown_chunker.rs`: a pure Markdown analyzer makes passages with heading path, UTF-8 byte offsets, 1-based inclusive body line interval, deterministic ordinal/hash, and bounded overlap. Heading/blank paragraph/list cuts score above arbitrary line breaks; fenced code is not split. A very large code fence can exceed the nominal chunk size; whole Markdown pages remain size-bounded by the existing vault.
- `database/knowledge_index.rs`: SQLx migration `knowledge/0002_markdown_passages.sql` stores chunk records and chunk FTS in the **existing** `knowledge_index.db`. Publish/rebuild updates page, all derived passages and index health in one SQLite transaction. Query returns one best passage per verified canonical Wiki ID before applying the result limit.
- The `knowledge.search` output retains stable `id/title/revision/excerpt/source_ref` and adds optional `start_line/end_line/heading_path`; existing frozen operation names/contracts and the separate verified line-read flow stay supported. Staged drafts do not enter the Agent passage index; Memory remains scoped, confirmed, and unchunked.
- Per-domain indexer versions: Knowledge 2, Memory 1. On migration from Knowledge 1, the version mismatch causes automatic rebuild of derived passages while preserving canonical Wiki and Source files. The index remains rebuildable, and failure closes reads as stale.

QMD inspired this algorithmic approach, not a code copy. See [comparison](qmd-markdown-implementation-review.md) and [ADR 0121](../adr/0121-qmd-inspired-markdown-passages.md). Embedding, sqlite-vec, RRF and LLM reranking remain explicitly unimplemented pending evaluation.

## Review remediation: publish outcome, Source health and Admin browsing (2026-10-18)

- Canonical publish and draft save return accepted Markdown records even if the rebuildable Knowledge index cannot reconcile; the index stays `stale` and the failure is logged. Admin checks `GET /api/admin/knowledge/index` before reporting search as ready.
- Ordinary Knowledge reconciliation checks every published immutable Source, including when the changed page is an unrelated draft. Startup does not switch to `ready` on invalid Source evidence.
- `knowledge.read` reports `truncated=true` if additional body lines exist even below the character cap.
- Admin search overlays staged revisions without exposing drafts to Agent FTS; staged matching is a case-insensitive lexical term filter, not BM25.
- Admin Wiki inventory accepts `after=<stable-id>` and returns optional `next_cursor` alongside existing `items`.
