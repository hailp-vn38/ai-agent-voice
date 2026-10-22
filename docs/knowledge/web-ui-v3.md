# Admin Web Knowledge and Memory V3

Keep separate top-level workspaces: **Knowledge** with Wiki Pages/Sources and **Memory** with Facts/Generated Summary. Do not merge Memory into Wiki or put both into Plugin settings. Use existing Vue admin shell and authenticated Admin HTTP. MVP: list/search/read/publish with revision conflict, source attribution, index health/rebuild; Memory requires explicit confirmation/correction/forget and indicates that MEMORY.md is generated. Reuse existing Job Detail Timeline rather than inventing an ingest Activity stream.

The original MVP did not include Quality/Activity/Review dashboards or vector/graph visualization; **Graph View has since been implemented** under ADR 0124. Direct file and index DB paths are never browser-controlled; return IDs and revision metadata. Admin routes do not bypass Rust Core validation. On degraded search, show the index status rather than stale search results. Future refinements may add Source preview, conflict compare and metadata graph with concrete product demand.

## Implemented minimal workspaces on this branch

Two separate Vue Admin pages at `/knowledge` and `/memory` reuse the authenticated API client and existing sidebar. Knowledge lists draft/published pages, supports full-text search, line-range reads and draft CAS creation/update. Memory lists all owner facts for Admin review (including unconfirmed/forgotten), while full-text recall remains confirmed/active only; it also supports editing, forget and generated summary. Both display independent index health/rebuild controls. They do implement bounded immutable text-Markdown Source import, explicit source-verified Wiki draft publication, and Admin-only Memory inventory (including unconfirmed facts). They do **not** implement PDF/media ingestion, Agent-assisted publication, AI Review Queue or vector retrieval. **An explicit Wiki Graph UI was added later under ADR 0124**.

The Knowledge workspace supports small text-Markdown Source imports, Source listing and explicit source-verified draft publication; raw binary media ingestion is not part of this slice. Agent tools must not list/read unverified drafts.

## Memory inventory and safety

Empty-query `GET /api/admin/memory/facts?limit=100` returns a bounded list of canonical facts for authenticated Admin review; full-text search with `q` continues to expose only confirmed, active facts. The Admin UI can open a pending fact by ID and set `confirmed=true`; it cannot reactivate a forgotten ID. Agent `memory.recall` and `memory.summary` never receive this inventory. This is not a physical-erasure or multi-tenant ownership system.

## Editing a published Wiki

Selecting a verified Wiki lets Admin save an edited **draft revision**, which stays separate from the accepted Wiki. The next Save sends the staged `expected_revision`; Publish verifies a Source reference and advances the canonical revision once. The Agent continues reading the last accepted revision until publication. Admin editor GET without line-range query reads the full, bounded Markdown document; partial line-range responses are not used in the save form.


## Approved physical-folder Explorer A (2026-10-18)

Knowledge now has three root panes displayed inside the existing /knowledge page: Raw, Wiki, Drafts. They are scanned as real directories under the Knowledge vault. Source snapshots in Raw are always visible but read-only; Raw working Markdown can be created/edited/imported explicitly. Wiki pages are saved at selected physical paths; canonical published revisions remain separate from Drafts, and the editor uses stable IDs for Wiki operations. The right-hand metadata pane shows physical path, revision or SHA-256 and Source reference.

The Admin-only tree response comes from GET /api/admin/knowledge/tree. Folder/file mutations pass root-qualified relative paths into Rust Core, never absolute host paths. Existing /knowledge/pages, /knowledge/sources and /knowledge/index endpoints remain valid; /memory and its policy remain unchanged. New folder operations are deliberately narrow: only create; directory move/delete/drag-and-drop remain deferred.

[ADR 0122](../adr/0122-knowledge-physical-folders.md) records why mutable Raw notes are separate from immutable Source snapshots and why no new path-catalog database is introduced.


## Obsidian-inspired folder note creation (2026-10-19)

Knowledge Explorer keeps the existing physical Raw/Wiki/Drafts root switch and reader/Graph layout. Each writable Raw or Wiki folder has a small **create Markdown note** action on hover/focus; the toolbar action targets the selected writable folder or the root. The filename is entered inline (Enter creates the file; Escape cancels); the .md extension is appended when omitted. Duplicate or unsafe names fail without overwriting documents. Immutable Source bundles and Drafts do not expose file-creation actions.

Creation uses existing Core/Admin endpoints: POST /api/admin/knowledge/raw/files for an empty mutable Raw note, or POST /api/admin/knowledge/pages for an empty Wiki draft at its physical path. The new note opens a full-height Markdown editor with filename breadcrumb, editable Markdown body, optional rendered preview and explicit save (Ctrl/Cmd+S). Raw edits use SHA-256 compare-and-swap; Wiki edits use stable ID and expected_revision and remain drafts. New Wiki notes are **not** published or exposed to Agent tools automatically: Source verification and Publish are separate operations. Existing notes may be reopened for editing; the normal reader is retained.

No new database, browser-controlled absolute paths, publish bypass, drag/drop directory management, or external Obsidian synchronization is added. The UI is inspired by Obsidian's file navigation/creation workflow, not full Obsidian feature parity.


## Current Knowledge workspace, Graph and dedicated Settings route (2026-10-19)

At `1d3f802`, `/knowledge` provides physical Raw/Wiki/Drafts source selection (browser preference persists), Reading/Compose/Graph modes, note-based Wiki/Raw creation from folder actions, explicit save (Ctrl/Cmd+S), Source/draft separate ownership, Markdown/Mermaid reader, Graph/backlinks via Cytoscape and graph-safe Wiki moves. Drafts are never a substitute for approved Wiki. The current template contains no visible immutable Source import/publish action despite existing Admin endpoints. Older sections in this file describe the baseline MVP and must not be mistaken for the current screen.

The vault-footer settings gear navigates to **`/knowledge/settings`**, a **separate preexisting page** (`KnowledgeSettingsPage.vue`) currently showing example-only Vault / Index & Graph / Publishing sections. It has no provider selector, persistence or settings HTTP endpoint. Future AI Knowledge Settings must evolve this existing route instead of adding a Settings root or tab to the physical Explorer.

Future AI Review Queue is likewise a **logical interface**, not a fourth vault space. It should coexist with existing Reading/Compose/Graph and compare the current verified Wiki, proposed draft and immutable Source evidence. AI proposals must not overwrite unsaved human Compose changes, publish human drafts silently, or treat the Admin Graph's draft nodes as verified content.

See [AI Ingest implementation](ai-ingest-auto-publish-implementation-v3.md), [branch audit](ai-ingest-branch-reconciliation-2026-10-19.md), [ADR 0124](../adr/0124-wiki-wikilinks-graph-projection.md) and [Graph implementation](wiki-graph-view-implementation-v3.md).
