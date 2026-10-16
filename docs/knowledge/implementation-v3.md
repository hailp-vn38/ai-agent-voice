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
| KnowledgeCore | publish/read/search/rebuild | trusted Wiki vault, source checks, revision history and Knowledge search index |
| MemoryCore | confirm/correct/forget/recall/summary/rebuild | owner-scoped canonical facts, generated summary and Memory search index |
| MarkdownVault (internal) | read/write(expected_revision)/scan | frontmatter validation, stable identity, lock, snapshot, atomic replacement |
| SearchIndex (internal) | rebuild/search/health | domain-specific SQL schema, filesystem/hash reconciliation and FTS5 |
| Admin adapter | authenticated Knowledge/Memory endpoints | delegates to the cores; no independent SQL/FS mutations |
| Job Core Plugin adapter (later) | frozen typed operations | delegates to same cores; no new runtime or tool registry |

## On-disk shape

```text
data/
  <main SQLite database path selected by database.url>
  knowledge/
    wiki/<stable-id>.md
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

Content hash lives in the derived index, never as a self-referential field inside Markdown. Stable IDs survive title and path changes. Unknown frontmatter must not be silently overwritten when editing imported documents. Citations reference immutable Source version/section, not only a Job Artifact.

## Safe write and recovery sequence

Validate domain/ID/scope/source/size; serialize per-vault writes; verify `expected_revision`; snapshot old revision; write to same-directory temporary file, flush and rename atomically; reconcile domain index; regenerate bounded Memory summary when Memory changed. Filesystem + SQLite cannot be committed atomically: accepted Markdown remains authoritative if FTS update fails. On start or explicit rebuild, compare indexed hashes against canonical files, do not mass-delete after missing root, parse errors or incomplete scan. Index ready means the last full scan succeeded.

Never return a stale `MEMORY.md` after a forget; if summary regeneration fails, report failure and suppress old summary. Historical revisions and external backups have independent retention; do not promise physical erasure without a separate purge policy.

## Staged acceptance

- P0: distinct Rust cores, canonical markdown writes/revision CAS, independent SQLx migrations, FTS search/recall, rebuild and tests.
- P1: hardened original Source import and provenance, Admin UI Pages/Sources/Facts/Summary with index diagnostics.
- P2: `core.knowledge` / `core.memory` as frozen operations in current Job V3 catalog, write permission gate, voice summary wiring and root/child ingest.
- P3: formats, optional FTS chunking, embeddings/reranking, optional Git sync, graph/quality flows.

Only label a stage complete when its acceptance is exercised, rather than inferring that migrations imply full product integration. See [database-layout-v3.md](database-layout-v3.md).
