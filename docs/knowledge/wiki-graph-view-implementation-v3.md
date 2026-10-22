# Knowledge V3 — Obsidian-compatible Wiki Graph View (implementation plan)

- **Status:** Implementation committed to target branch; CI/build and Obsidian interactive smoke verification pending. See the tests and flow below; do not mark release-accepted until checks pass.
- **Target:** `feat/knowledge-memory-v3-file-first-index` (reviewed at `978ac5f0`, 2026-10-19).
- **Decision:** [ADR 0124](../adr/0124-wiki-wikilinks-graph-projection.md)
- **Flow:** [Knowledge Graph flow](../flows/14-knowledge-graph-v3.md)
- **Preserve:** [ADR 0118](../adr/0118-knowledge-storage-revisions-provenance.md), [ADR 0119](../adr/0119-memory-wiki-separated-ownership.md), [ADR 0120](../adr/0120-three-sqlite-database-lifecycles.md), [ADR 0121](../adr/0121-qmd-inspired-markdown-passages.md), [ADR 0122](../adr/0122-knowledge-physical-folders.md).

## 1. Goal and hard requirements

Implement an Obsidian-like **Global Graph, Local Graph, Outgoing Links and Backlinks** inside existing Knowledge Web UI while ensuring the very same `knowledge/wiki/**/*.md` notes produce a functioning graph when opened as an Obsidian Vault. **The link must exist in Markdown**; a row in SQLite alone is insufficient for Obsidian.

Do **not** create a second knowledge owner, graph database, fourth SQLite database, virtual folder tree, additional Agent Job operation, or new Wiki approval workflow. Keep existing Root selector (Raw / Wiki / Drafts), page editor, physical paths and status/provenance display. Graph remains authenticated **Admin-only** in this phase; Memory, Raw Sources and Job V3 are out of graph scope.

### Outcomes

1. Editing a Wiki body can create ordinary `[[AI/LLM]]` or `[[Rust/Async|Rust concurrency]]` references using an Insert Wiki Link picker. Saved Markdown is directly consumable by Obsidian.
2. Knowledge Graph visualizes references between physical Wiki Markdown files. Clicking a node opens the existing Wiki editor; clicking an edge identifies the source, target and line(s).
3. A page's Local Graph shows one- and optionally two-hop neighbors; the details panel shows incoming/outgoing counts and accessible unresolved-link warnings.
4. Moving a Wiki cannot silently break Obsidian links. The MVP rejects an unsafe move with actionable incoming-reference diagnostics rather than silently rewriting accepted third-party Wiki revisions.
5. On restart/rebuild the graph can be reproduced exactly from canonical Wiki Markdown, even after discarding `knowledge_index.db`.

## 2. Verified repository seams (2026-10-19)

| File | Current behavior | Required change |
|---|---|---|
| `src/services/knowledge/vault.rs` | File-first canonical `wiki/` scan; stable ID from strict YAML; nested paths; `Record.body`; revision CAS | Reuse existing scan and body; **do not** store graph or unknown keys in frontmatter |
| `src/services/knowledge/wiki.rs` | `admin_list_page` scans physical files; `put_draft_at`, `publish`, `move_page`; verified Source check | Expose one narrow graph read; guard unsafe moves using canonical links and path rules |
| `src/services/knowledge/browser.rs` | Root-qualified physical tree `raw/wiki/drafts`, scoped by `space` | Preserve this tree; graph cannot replace it |
| `src/database/knowledge_index.rs` | `knowledge_index.db`, schema v2, `kb_documents`, `kb_fts`, `kb_chunks`, same-transaction reconcile/rebuild | Extend existing Knowledge projection to derive `kb_links` transactionally |
| `migrations/knowledge/` | `0001_index.sql`, `0002_markdown_passages.sql` | Add **`0003_wikilink_graph.sql`**, Knowledge indexer version **3** |
| `src/app/admin/knowledge.rs`, `src/app/admin/mod.rs` | Authenticated Admin adapter / Knowledge routes | Add a single graph read route and typed query; no direct SQL/FS in adapter |
| `apps/admin-web/src/pages/knowledge/KnowledgePage.vue` | Three-pane Knowledge Explorer, Wiki edit/preview, source-verified publish, physical move | Add Graph tab/surface within the page; reuse existing selection/navigation/editor |
| `apps/admin-web/package.json` | Vue 3; no graph renderer today | Add one mature renderer (`cytoscape`, with TS types if necessary), not custom physics |
| `docs/knowledge/database-layout-v3.md` | MVP originally excluded `kb_links` | This plan is a **subsequent feature**, not a retroactive statement that MVP already included it |

Existing `vault::parse/encode` accept a fixed set of frontmatter keys (`id`, `kind`, `title`, `revision`, `status`, `scope`, `confirmed`, `source_ref`, `draft_base_revision`). Graph relationships therefore live in the **Markdown body** only; no `links:` field is added to `Record` or YAML.

## 3. Canonical link contract for Web and Obsidian

**Obsidian Vault root:** open `<main-db-parent>/knowledge/wiki/` as the Vault (not `knowledge/`). Keep `drafts/`, `raw/`, revision history, immutable Sources and SQLite outside the Obsidian Vault. Native Obsidian may create `wiki/.obsidian/`; canonical Wiki scanner and Admin Explorer ignore only this root-level configuration directory after rejecting symlinks, leaving all regular Wiki Markdown scanning and safety rules intact.

Use a real file path from the Vault root, with `.md` omitted, for links generated by the Web picker:

```text
knowledge/wiki/
  AI/Agent.md
  AI/LLM.md
  Rust/Async.md
```

```markdown
# AI Agent
Reasoning uses [[AI/LLM]].
Runtime uses [[Rust/Async|asynchronous Rust]].
See [[AI/LLM#Inference]] for the section.
```

**Supported graph-reading syntax (v1):**

- Wikilinks `[[path/to/Note]]`, `[[path/to/Note|alias]]`, anchors `[[path/to/Note#Heading]]` (edge points to the note, heading retained for navigation/display), and embeds `![[path/to/Note]]` (edge kind `embed`).
- Media and document embeds, such as `![[photo.png]]`, `![[voice.mp3]]` or `![[document.pdf]]`, do **not** become Wiki edges or unresolved Wiki references.
- Standard local Markdown note links `[label](../Rust/Async.md)` and `[label](AI/LLM.md#Inference)` (relative to **source file directory**, per Markdown semantics); absolute HTTP(S), URL-only links, fragments-only and asset links are not Wiki edges.
- Do **not** treat mentions, tags, shared titles, semantic similarity, provenance `source_ref`, or frontmatter `id` values as links.

**Resolution (shared internal rules):**

1. Keep both **stable Wiki identity** (`Record.id`) and physical normalized Vault-relative path (`AI/LLM.md`). The latter is the Obsidian identifier; stable ID is the database/UI identifier. Never generate `[[wiki_<uuid>]]` unless that is the physical basename.
2. For Wikilinks, normalize `AI/LLM` to `AI/LLM.md` relative to the Vault root. A basename-only `[[LLM]]` resolves only when exactly one file named `LLM.md` exists, including across directories. A duplicate basename is `ambiguous`; do not select an arbitrary destination. Do not apply `title` frontmatter as an automatic path alias.
3. For local Markdown links, resolve `.`/`..` relative to the source's physical folder, then require the resolved path to remain within `wiki/`. Reject absolute paths, external schemes, traversal outside the Vault, and symbolic-link destinations.
4. Strip heading/block fragments for note identity; retain them as link metadata. Decode percent-encoded local Markdown paths safely. Check file path case exactly, independent of a case-insensitive host filesystem. Reject ambiguous/unsafe generated path characters (e.g. `#`, `[`, `]`, `^`) in the **Insert Link** picker, without invalidating preexisting readable Wiki records.
5. Scan valid Markdown contexts only: exclude YAML frontmatter, fenced/indented code, inline code, escaped delimiters and ordinary Markdown link text that is not a link. Resolve links without modifying the original body. The parser is a private pure module, not ad-hoc regular expressions scattered between Vue and Rust.
6. Missing target -> `missing`; basename collision -> `ambiguous`; escaping/invalid target -> `invalid`. Diagnostics are Admin-only and do not become synthetic valid Wiki nodes. No implicit file creation.

Prefer the complete Vault-relative link generated by Web. The resolver intentionally accepts additional valid Obsidian/Markdown shorthand for existing notes, while **documenting** that exact parity for every Obsidian plugin/custom resolver is out of scope.

## 4. Deep module design and placement

**External seam:** `KnowledgeCore.graph(query) -> GraphSnapshot` (Admin-only), reused by Global/Local Graph, link details and tests. `KnowledgeCore.move_page` retains its existing interface and adds an inbound-link integrity guard. This concentrates graph ownership and does not expand Agent-facing Core Plugin tools.

**Internal implementation:**

- `services/knowledge/wiki_links.rs` — pure Markdown link extraction + deterministic path resolution against a complete `WikiPathCatalog` built from the existing vault scan; small testable parser/resolver interface, no DB.
- `database/knowledge_index.rs` — graph rows, transactional reconciliation, health/version, and bounded read queries. Keep graph SQL inside the existing `SearchIndex` implementation without introducing a separate graph repository or speculative layer.
- `app/admin/knowledge.rs` — thin authenticated adapter; typed query and response, no graph logic.
- `KnowledgePage.vue` + `KnowledgeGraph.vue` — Vue view/rendering adapter; graph layout/zoom and node selection stay client side. The Web picker reuses existing `GET /api/admin/knowledge/tree?space=wiki` nodes rather than maintaining a second target registry; only the internal Rust resolver builds a path catalog.

**Deletion test:** removing `wiki_links` would reintroduce parsing, target resolution, ambiguity, rename-safety and line offsets across index, UI and tests. Therefore this is a deep module rather than two parallel graph parsers.

## 5. SQLite projection and migration

Add `crates/voice-agent-server/migrations/knowledge/0003_wikilink_graph.sql`:

```sql
CREATE TABLE kb_links (
    source_id TEXT NOT NULL REFERENCES kb_documents(stable_id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL CHECK(ordinal >= 0),
    source_line INTEGER NOT NULL CHECK(source_line > 0),
    raw_target TEXT NOT NULL,
    target_id TEXT REFERENCES kb_documents(stable_id) ON DELETE CASCADE,
    link_kind TEXT NOT NULL CHECK(link_kind IN ('wikilink','embed','markdown')),
    resolution TEXT NOT NULL CHECK(resolution IN ('resolved','missing','ambiguous','invalid')),
    PRIMARY KEY (source_id, ordinal),
    CHECK ((resolution = 'resolved') = (target_id IS NOT NULL))
);
CREATE INDEX kb_links_target ON kb_links(target_id)
    WHERE target_id IS NOT NULL;
```

`kb_documents` remains the canonical metadata *projection* of Wiki files. When a link needs its source revision, read `kb_documents.revision` through `source_id`; do not duplicate it in `kb_links`. `kb_links` is a **derived** graph projection in the same `knowledge_index.db`. No `kb_nodes` table: nodes come from `kb_documents`. Do not create `kb_links` in the application DB or Memory DB. Do not duplicate Markdown bodies.

Graph index version changes **2 -> 3**. Knowledge-only migrations and FTS/passages still migrate independently from Memory and main SQLx database. In a complete successful scan:

1. Snapshot all canonical Wiki records and paths using existing `MarkdownVault.scan()`; validate unique stable IDs, file safety, full parse, and source requirements in the current calling flow.
2. Parse links from canonical `Record.body`, build catalog and resolve globally. Re-resolve **all** links when paths change, even if the source body hash did not change: target rename/creation can change resolutions.
3. In one Knowledge index transaction: reconcile `kb_documents`, `kb_fts`, `kb_chunks`, `kb_chunks_fts` as now; replace the derived `kb_links` projection for this complete snapshot; mark `kb_index_state.health='ready'` and `indexer_version=3`; commit.
4. On parse/scan/index failure, **rollback** the projection and keep health stale/error. An index rebuild forces recreation of links and FTS from Markdown. Missing Vault/mount/symlink/duplicate ID must never prune existing rows.
5. A path move with unchanged source body still causes updated catalog resolution and the relevant graph changes on the next reconcile. On first delivery use the already-bounded full scan and full link replacement for correctness; optimize changed-document extraction only after a measured need (do not add a second cache/ledger).

**Important:** if SQLx foreign keys are enabled, ensure `kb_documents` upsert/deletion ordering and link replacement are compatible. Clear existing `kb_links` before retiring `kb_documents`, then insert links after all current document rows exist, in the same transaction.

## 6. Graph visibility and integrity policy

- **Admin Graph** includes the canonical files under `wiki/`: both `verified` and initial `draft` Wiki pages, each with a visible `status`. For a verified page with a pending staged revision in `drafts/`, graph reads **only the last accepted canonical body**, not the pending draft.
- **Agent Job** continues using `knowledge.search/read_verified` and Source checksum verification; adding `kb_links` gives it **no** automatic graph tool, draft access, traversal or derived trust. Graph links cannot promote a draft to verified.
- Search, graph and backlinks never read Raw, Source bundles, history or Memory content as Wiki nodes.
- If Knowledge index is stale/unavailable, the graph read returns a consistent error (HTTP **503**, `index_stale` or current service code) rather than presenting stale links as current. No partial filesystem fallback inside the Web adapter.
- Keep `source_ref`, revision, metadata, frontmatter parser, existing frozen Job V3 operation names and the canonical publish CAS unchanged.

## 7. Safe Wiki move and external Obsidian editing

Existing `KnowledgeCore.move_page()` changes the physical path without rewriting Markdown backlinks. Graph support **must not** claim a move is safe merely because the stable ID is unchanged.

**MVP move policy (deliberately conservative):**

- Before move require a healthy reconciled catalog. Simulate target path and resolve the **whole canonical graph** against the prospective catalog. Reject `409 wiki_move_breaks_links` if any currently resolved edge becomes unresolved or changes destination (including basename collision), including links in the moved note itself. Return affected `source_id`, `source_path`, `source_line`, `raw_target` (bounded) for the UI to explain the block. Continue enforcing existing expected revision/folder and no staged-draft move rule.
- Users can update the affected referencing notes through normal save-draft/publish CAS and Source verification, then retry. No background wholesale rewrite of other verified pages and no silent revision increment.
- After a successful physical move, mark index stale and reconcile as the existing code does. A reconciliation failure preserves the moved canonical path and degrades graph/search consistently; the response should not imply that the old path was kept.
- Resolve against **prospective** file paths, not against stable IDs alone. A rename cannot be considered link-safe merely because `stable_id` is still present.

Opening `knowledge/wiki/` in Obsidian as a **read-only Viewer** works immediately once Markdown links exist. Do **not** promise bidirectional write synchronization: Obsidian can modify body/frontmatter/filenames without server revision CAS, staged approval or provenance verification. For MVP, document **read-only Obsidian usage**, or work on a separate copy. Writable Obsidian import/merge/watch with authorization and CAS is a distinct future decision, not smuggled into Graph View scope.

## 8. Authenticated Admin HTTP contract

New route under the existing Admin router:

```http
GET /api/admin/knowledge/graph?scope=global&limit=500
GET /api/admin/knowledge/graph?scope=local&center_id=wiki_...&depth=1
```

Validation: `scope=global|local`, valid stable ID for Local, `depth=1|2`, `limit=1..500`; cap raw relationships at 2,000 and return `truncated: true` when results are limited. Sort deterministically (physical path, then IDs); for Local prioritize center and closest neighbors before truncation. Scope and center ID stay in the request; index health is represented by HTTP 200/503. Response shape:

```json
{
  "nodes": [
    {"id":"wiki_agent","path":"wiki/AI/Agent.md","title":"Agent","status":"verified","revision":3},
    {"id":"wiki_llm","path":"wiki/AI/LLM.md","title":"LLM","status":"verified","revision":1}
  ],
  "edges": [
    {"source_id":"wiki_agent","target_id":"wiki_llm","kind":"wikilink","lines":[8],"references":[{"line":8,"raw_target":"AI/LLM#Inference|Language Model","alias":"Language Model","anchor":"Inference"}]}
  ],
  "unresolved": [
    {"source_id":"wiki_agent","source_line":15,"raw_target":"Missing","reason":"missing"}
  ],
  "truncated": false
}
```

Aggregate repeated resolved links per source/target/kind; `lines` records source line numbers for each occurrence (repeats are allowed) and `lines.length` is the displayed occurrence count. The optional-detail `references[]` array retains the original per-occurrence target, alias and anchor from Markdown for edge inspection; it is rebuildable, not a separate link authority. Bound `unresolved` with the same response limits and mark `truncated: true` whenever any node, edge, line occurrence or diagnostic is omitted. Do not expose SQL row IDs, filesystem absolute paths, or user-controlled arbitrary table/column names.

**Route is Admin-only.** Reuse existing authentication/error translation. No Graph mutation HTTP route; the sole mutation is the existing Wiki editor saving Markdown, publishing it through current checks.

## 9. Web UI design and interaction

Keep `/knowledge` and its three-pane physical Explorer. Inside the center workspace introduce a segmented **Editor | Graph** view, available when Wiki space is active. Graph UI has:

- **Global / Local** toggle; Local centered on the currently selected Wiki; neighborhood depth 1 default, 2 optional.
- Pan, zoom, fit, re-layout, hover title/path; click node -> existing `openPage(id)` and switch to editor on explicit Open action (do not force navigation on hover).
- Node status distinguish verified/draft; highlights for selected/neighbor. Edges are directional in data, may render as simple lines for readability; tooltip lists source -> target and link locations.
- A compact **Backlinks / Outgoing / Unresolved** panel on selected Wiki, derived from the same graph response. Local retrieval should include incoming and outgoing neighbors. Separate read of selected page is still the source for the editor content.
- **Insert Wiki Link** picker next to the existing Markdown textarea: select from the existing Wiki physical tree, strip the `wiki/` root and `.md` suffix (e.g. `wiki/AI/LLM.md` -> `[[AI/LLM]]`), then insert at cursor; optionally add a display alias. Do not materialize edges directly in SQLite. Show full physical paths to distinguish duplicate filenames.
- An index-stale state with Retry/Rebuild action, large-graph truncation notice, empty states and keyboard-accessible list alternative to pointer-only Canvas. Localization keys in existing i18n.
- Refresh graph after save, publish, safe move or explicit index rebuild; retain current UI selected ID. `raw`/`drafts` views and Source panel keep present behavior. No change to primary navigation.

**Rendering adapter:** add `cytoscape` as one purpose-built Graph UI dependency rather than implementing physics, hit-testing, zoom and event handling manually. Use GraphSnapshot as the only input; destroy/dispose renderer on unmount. Avoid graph render work during ordinary Editor-only interactions.

## 10. Acceptance tests

### Pure link parser/resolver
- `[[AI/LLM]]`, `[[AI/LLM|alias]]`, `[[AI/LLM#Heading]]`, `![[AI/LLM]]`; Markdown relative links, UTF-8 titles, percent-encoding, multiple links on one line, repeated edges, stable line offsets.
- Exclude YAML frontmatter, fenced and inline code, escaped literal `\\[[...]]`, remote links, non-Markdown attachments; prevent traversal and symlinks.
- Basename `[[LLM]]` unique -> resolved; duplicate `AI/LLM.md` + `Other/LLM.md` -> ambiguous; full path remains unambiguous; missing -> diagnostic. Target IDs remain stable across a safe move, but their physical lookup changes.
- Limits, malformed syntax, Unicode file names and exact path/case handling.

### Persistence and integrity
- New migration on existing v2 Knowledge index; no change to application/Memory migrations or schema.
- Fresh create -> save draft -> verified publish -> staged new edit: Graph reflects **canonical** content at each point; staged changes invisible until publish.
- Index rebuild from files reproduces the same graph; changing a target path or deleting a file re-resolves unchanged incoming link text.
- Failed complete scan/SQL transaction preserves previous derived rows but health is stale; HTTP graph must fail closed. No orphan graph rows on deletion.
- Unsafe move (inbound path-dependent links, outgoing relative Markdown links, basename collision) returns 409 and leaves filesystem/history/index unchanged; safe move succeeds and reconciles.
- Existing Agent tools still refuse drafts/unverified Sources and have unchanged schemas.

### HTTP/Web
- Auth required; bounded local and global queries; 400 for invalid input, 404 for missing center, 503 for stale index.
- Vue Graph local/global rendering, node -> page editor, backlinks/outgoing, no hidden draft exposure, refresh after publish/move, empty/truncated/failed states and keyboard alternative.
- `npm run typecheck`, `npm run test`, `npm run build`, `cargo test -p voice-agent-server`, format/lint/CI as available.

### Real Obsidian smoke gate (manual, essential)
1. Use Web to create `wiki/AI/LLM.md` and `wiki/AI/Agent.md`, adding `[[AI/LLM]]` via picker to Agent body.
2. Publish via existing Source-verified flow; confirm Web Global/Local/Backlinks show `Agent -> LLM`.
3. Open **the physical `knowledge/wiki/` directory as an Obsidian Vault** (read-only copy acceptable). Confirm native Obsidian Graph and Backlinks show the **same edge without importing SQL or running the Rust graph route**.
4. Test `[[AI/LLM|alias]]` and `[[AI/LLM#Heading]]`. Validate that a blocked unsafe rename does not produce an apparently successful move.
5. Verify opening the Vault does not expose `drafts/`, `raw/`, `memory/`, `revisions/` or SQLite as normal wiki notes.

## 11. Implementation slices (in dependency order)

1. **T1 — Pure parser/resolver.** Build `wiki_links.rs` and path catalog; establish Obsidian-compatible link contract by unit tests. No UI work first.
2. **T2 — Derived index.** Add migration 0003, version bump, transactional graph projection/rebuild with integrity/health tests; no new DB.
3. **T3 — Core and HTTP.** Implement one Admin graph interface/route with bounded global/local result; add unsafe-move guard using same resolver.
   - Resolve one/two-hop neighbor IDs around `center_id` before limiting Local Graph nodes/edges; prioritized center-incident edges remain visible under the occurrence cap.
4. **T4 — Vue integration.** Graph tab, existing Wiki editor navigation, backlinks/outgoing/unresolved details, Insert Wiki Link picker and refresh/error handling.
5. **T5 — Cross-app validation.** Rust/TS/HTTP tests, real Obsidian Vault smoke and documentation acceptance. Ship only after graph and Obsidian agree.

**Explicitly deferred:** semantic similarity/AI-inferred edges; generic graph editing; Memory graph; cross-vault links; full Obsidian plugin API/alias compatibility; writable Obsidian sync; background filesystem watcher; auto-fixing historical broken links; moving entire directories; realtime collaboration; native Graph persistence to the application DB.

## 12. Alternatives considered

See [ADR 0124 — Alternatives rejected](../adr/0124-wiki-wikilinks-graph-projection.md#alternatives-rejected).
