# Knowledge V3 — Branch change audit for AI Ingest documentation

**Audit date:** 2026-10-19.  
**Target:** `feat/knowledge-memory-v3-file-first-index` at commit `1d3f80282288f33417094c91660338c3fb176033`.  
**Comparison baseline:** `08bafa62248faadc4b0104ea9c2c8c66d0713120` (previous AI Ingest document-index update).  
**GitHub comparison:** https://github.com/hailp-vn38/ai-agent-voice/compare/08bafa62248faadc4b0104ea9c2c8c66d0713120...1d3f80282288f33417094c91660338c3fb176033

GitHub reports **145 commits ahead, 0 behind**, with changes across Knowledge Rust Core, Knowledge index, Web/graph, documentation and UI tests. This is a **code-and-contract review**, not execution of Rust/TypeScript tests. Later commits may require another audit.

## A. What has actually changed

| Area | Code evidence | Current behavior | Implication for planned AI Ingest |
|---|---|---|---|
| Physical vault | `services/knowledge/browser.rs`; `KnowledgePage.vue` | Real Raw/Wiki/Drafts roots; scoped `GET /api/admin/knowledge/tree?space=...`; in-folder Raw/Wiki Markdown creation; immutable Source bundles are read-only. | Analyze only a committed, immutable Source snapshot; a Raw working note is not automatically Source. Do not invent a second file browser or virtual folders. |
| Wiki authoring | `KnowledgePage.vue`, `knowledge-reader.css`, `markdown.ts` | Obsidian-inspired Reading/Compose/Graph views, inline filename creation, explicit save and dirty editor guard; Wiki creation stays draft. | Review must preserve pending human changes and use draft/revision CAS; no automatic overwrite of open/pending drafts. |
| Web persistence | `KnowledgePage.vue` | Remembers selected root and expanded paths/selected notes per vault in browser storage. | Reuse selection/navigation conventions; processing status belongs in Job/Review metadata, not browser-only state. |
| Knowledge Settings placeholder | `KnowledgeSettingsPage.vue`, `router/index.ts`, `KnowledgeSettingsPage.test.ts` | Dedicated route `/knowledge/settings`, reachable via gear in vault footer. Page explicitly says it has examples only, **no persisted config/endpoints/form controls**. | Implement real Knowledge AI/provider settings on this route; do **not** add a Settings tab to Raw/Wiki/Drafts. |
| Markdown reader | `markdown.ts`, `readingMarkdown.ts`, `mermaidRenderer.ts` | Sanitized Markdown render, Wiki link click navigation, outline, optional Mermaid rendering. | Show AI proposal as a read-only Markdown preview using existing rendering and safe escaping; avoid new raw HTML rendering. |
| Wiki relation authority | `services/knowledge/wiki_links.rs` | Extracts and resolves Wikilinks, embeds and local Markdown links from canonical Wiki bodies; handles paths, ambiguity, missing targets, some ignored Markdown contexts. | AI-generated links must be Vault-relative real Markdown links. Check resolution when assessing Auto Publish; never store relationships only in a new SQL table. |
| Graph storage | `migrations/knowledge/0003_wikilink_graph.sql`; `database/knowledge_index.rs` | New **`kb_links`** derived table in the **existing** `knowledge_index.db`; indexer version 3; FTS, passages and links reconcile from canonical Markdown in one transaction. | Do not create Knowledge relation DB or a separate graph writer. A successful publish needs the existing index reconciliation; failure degrades search/graph but does not undo Markdown. |
| Graph read | `KnowledgeCore::graph`; `GET /api/admin/knowledge/graph`; `KnowledgeGraph.vue` | Admin global/local Graph; 1/2-hop traversal, bounded nodes/edges, backlinks/outgoing/unresolved diagnostics; GraphView uses Cytoscape. | Graph is a *derived view*, not the claim-evidence gate. Reuse parser/resolver logic internally where appropriate; do not use Graph admin output as authority to publish. |
| Graph visibility | `database/knowledge_index.rs`; ADR 0124 | Graph projection scans canonical `wiki/` including **new canonical drafts**; staged replacements under `drafts/` do not replace accepted revision/edges. Agent `knowledge.read/search` remain verified-only. | A linked node that exists in Admin Graph could still be unverified. Auto Publish must require verified Wiki targets when the link is relied upon; ambiguous/missing/unverified links route to Review. |
| Move safety | `KnowledgeCore::move_page`, `wiki_links::move_conflicts` | Checks link-target continuity and index health; unsafe changes return `wiki_move_breaks_links` / HTTP 409 with affected links. | Auto-generated updates must use stable Wiki ID plus current physical path/revision. Never rename/move existing Wiki pages to satisfy an AI link without explicit review. |
| Knowledge Graph docs | `docs/adr/0124-*`, `docs/flows/14-knowledge-graph-v3.md`, `docs/knowledge/wiki-graph-view-implementation-v3.md` | Graph implementation committed; real native-Obsidian smoke test documented but not shown as passed. | AI Ingest design must link ADR 0124, preserve Vault root `knowledge/wiki/` and treat smoke as a release gate, not certified interoperability. |
| Admin navigation | `apps/admin-web/src/pages/jobs/JobsPage.vue`, `navigation.ts` | Background Settings moved into Jobs UI; Knowledge sidebar settings gear exists independently. | Knowledge provider choices should not modify Global Job settings or restore the obsolete Background settings navigation. |

## B. What has **not** been implemented by this branch delta

As of the inspected commit:
- No `knowledge_ingest_v3` mode in production Job admission/scheduler/execution.
- No `KnowledgeProcessing` module or deterministic AI proposal policy.
- No persisted `knowledge_processing_settings` or `knowledge_proposals` main-DB migrations, Knowledge AI Settings endpoints or provider selector.
- No Source `/analyze` endpoint, AI-origin proposal lifecycle, Review Queue, or automatic publication of machine-produced Wiki.
- No optional independent AI verifier.
- Existing Markdown Source import/draft/manual `publish` remains the only supported path.

These are **planned** work in [the AI Ingest implementation specification](ai-ingest-auto-publish-implementation-v3.md), not features enabled by the new graph/Settings UI commits.

## C. Required adjustments to AI Ingest implementation

1. Keep Knowledge Settings at `/knowledge/settings`, upgrading `KnowledgeSettingsPage.vue` instead of replacing/duplicating it. Reuse current vault-footer gear; no sidebar tab needed.
2. Treat **Review as a logical queue/view** owned by KnowledgeProcessing. Do not create a physical `review/` vault root, silently co-opt `drafts/` or replace Raw/Wiki/Drafts. The Review UI can be a contextual tab/view within `/knowledge` but must not compete with physical-space selector or Graph mode.
3. Preserve the existing Reading/Compose/Graph views and human dirty-editor guard. AI proposals must not force-close a human Compose session or silently publish unsaved editor text.
4. Add graph/link assessment to candidate validation, using canonical Wiki path resolver and verified status. Explicit or newly generated Wikilinks that resolve ambiguously, are broken, point only to unverified drafts, or require renaming a target ⇒ pending Review. Missing references that are explicitly deliberate must still go to Review, not auto silently.
5. Allow only **existing verified Wiki** references as positive link targets for first-slice Auto Publish. Link relationships to simultaneously generated candidate pages and preexisting unverified drafts require later multi-proposal admission and transaction planning.
6. Use `knowledge/wiki/` as the Obsidian Vault root. Generated links use Vault-relative file paths without extension (`[[AI/LLM]]`), not Rust stable IDs or main-DB relation rows. An AI proposal cannot unilaterally move files or rewrite other backlinks.
7. Index reconciliation now covers FTS + passages + `kb_links` in `knowledge_index.db`. Publishing intent recovery must not claim Graph ready if the index is stale; keep canonical publish and surface repair.
8. Keep Source import separate from Raw note save. `auto_after_import` only fires on a successful immutable Source snapshot commit, once per pinned source version/policy idempotency key.
9. Do not relax the existing Job V3 ProviderPin or introduce provider switching mid-flight. `KnowledgeSettingsPage.vue` gets its own provider selector after qualified backend endpoints exist.

## D. Acceptance additions

- Rust: valid/invalid/ambiguous `[[wikilinks]]`; draft target rejected for auto; missing links requiring review; staged replacement retains graph; publish updates `kb_links`; unhealthy index blocks positive auto decision; title/path reuse and link resolution; unsafe move returns typed 409.
- Web: Knowledge settings gear navigates to the existing route; new controls load/save via expected-revision CAS; Raw/Wiki/Drafts controls remain unchanged; Reading/Compose/Graph survive; Review UI preserves unsaved text/selected vault; proposed links preview correctly.
- Regression: existing `wiki_links`, `wiki_graph`, `local_graph`, `KnowledgePage.test.ts`, `KnowledgeSettingsPage.test.ts`; existing Agent read policy; Root/Child + plugin-only Job V3; Memory.
- Manual: execute `docs/knowledge/obsidian-graph-smoke-v3.md` on a **copy** of the Wiki Vault. Do not claim native Obsidian certification from unit tests alone.
- Verification: re-run Rust, Web build/test, GitHub Actions and branch-HEAD checks after the implementation commits. This audit did not execute tests.

## E. Documentation synchronized

- [AI Ingest / Review / Auto Publish implementation](ai-ingest-auto-publish-implementation-v3.md): updated code seam and UI/Graph rules.
- [ADR 0123](../adr/0123-knowledge-ingest-job-execution-and-review-policy.md): reconciled with ADR 0124 and new Knowledge Settings route.
- [Knowledge V3 README](README.md): linked the present audit and current indexing/version notes.
- [Knowledge flow 13](../flows/13-knowledge-v3.md): current graph dependency and explicitly future AI path.

**Non-negotiable:** this audit preserves the accepted ownership model: KnowledgeProcessing owns decisions, Agent Job V3 owns execution, KnowledgeCore owns durable publication, and all Wiki relationships derive from canonical Markdown.
