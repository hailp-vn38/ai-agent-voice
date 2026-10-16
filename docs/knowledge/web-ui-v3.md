# Admin Web Knowledge and Memory V3

Keep separate top-level workspaces: **Knowledge** with Wiki Pages/Sources and **Memory** with Facts/Generated Summary. Do not merge Memory into Wiki or put both into Plugin settings. Use existing Vue admin shell and authenticated Admin HTTP. MVP: list/search/read/publish with revision conflict, source attribution, index health/rebuild; Memory requires explicit confirmation/correction/forget and indicates that MEMORY.md is generated. Reuse existing Job Detail Timeline rather than inventing an ingest Activity stream.

No new Quality/Activity/Review dashboards or vector/graph visualization in MVP. Direct file and index DB paths are never browser-controlled; return IDs and revision metadata. Admin routes do not bypass Rust Core validation. On degraded search, show the index status rather than stale search results. Future refinements may add Source preview, conflict compare and metadata graph with concrete product demand.

## Implemented minimal workspaces on this branch

Two separate Vue Admin pages at `/knowledge` and `/memory` reuse the authenticated API client and existing sidebar. Knowledge lists draft/published pages, supports full-text search, line-range reads and draft CAS creation/update. Memory lists all owner facts for Admin review (including unconfirmed/forgotten), while full-text recall remains confirmed/active only; it also supports editing, forget and generated summary. Both display independent index health/rebuild controls. They do implement bounded immutable text-Markdown Source import, explicit source-verified Wiki draft publication, and Admin-only Memory inventory (including unconfirmed facts). They do **not** implement PDF/media ingestion, Agent-assisted publication, speculative Quality/Activity/Review dashboards, or graph/vector UI.

The Knowledge workspace supports small text-Markdown Source imports, Source listing and explicit source-verified draft publication; raw binary media ingestion is not part of this slice. Agent tools must not list/read unverified drafts.

## Memory inventory and safety

Empty-query `GET /api/admin/memory/facts?limit=100` returns a bounded list of canonical facts for authenticated Admin review; full-text search with `q` continues to expose only confirmed, active facts. The Admin UI can open a pending fact by ID and set `confirmed=true`; it cannot reactivate a forgotten ID. Agent `memory.recall` and `memory.summary` never receive this inventory. This is not a physical-erasure or multi-tenant ownership system.

## Editing a published Wiki

Selecting a verified Wiki lets Admin save an edited **draft revision**, which stays separate from the accepted Wiki. The next Save sends the staged `expected_revision`; Publish verifies a Source reference and advances the canonical revision once. The Agent continues reading the last accepted revision until publication. Admin editor GET without line-range query reads the full, bounded Markdown document; partial line-range responses are not used in the save form.
