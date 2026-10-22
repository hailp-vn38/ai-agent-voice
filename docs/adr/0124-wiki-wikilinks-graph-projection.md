# ADR 0124 — Wiki links are canonical Markdown; Graph View is a derived Knowledge index

- Status: **Accepted for implementation** (code committed; CI and native Obsidian smoke pending)
- Date: 2026-10-19
- Scope: Knowledge V3 Wiki, Admin Graph View, Obsidian interoperability
- Related: [ADR 0118](0118-knowledge-storage-revisions-provenance.md), [ADR 0119](0119-memory-wiki-separated-ownership.md), [ADR 0120](0120-three-sqlite-database-lifecycles.md), [ADR 0121](0121-qmd-inspired-markdown-passages.md), [ADR 0122](0122-knowledge-physical-folders.md)
- Implementation: [Knowledge Graph implementation plan](../knowledge/wiki-graph-view-implementation-v3.md)

## Context

Knowledge V3 stores canonical Wiki content as frontmatter Markdown in real nested directories under knowledge/wiki/, with stable identity independent from filename/path. A separate knowledge_index.db holds rebuildable document/chunk/FTS projections. The Admin Web UI uses the physical tree and an existing Markdown editor. Obsidian's native Graph View recognizes Markdown links, not relationships stored only in SQL. The current Wiki move implementation retains stable ID but does not rewrite incoming physical-path Wikilinks; leaving that unchanged without guarding would silently break Obsidian relationships.

## Decision

1. **Canonical link authority is the text in the Wiki Markdown body**: Obsidian-compatible [[Vault/Relative/Note]], its optional alias/heading, embeds, and local Markdown note links. IDs, titles and Source references cannot substitute for physical-path links.
2. **Single derived graph index**: add kb_links to the existing knowledge_index.db (new Knowledge-only migration 0003, indexer v3), alongside kb_documents, kb_chunks, and FTS. Rebuild/reconcile from canonical Wiki in one SQL transaction. No extra DB, sidecar, root-level map, Memory relation or graph-specific write endpoint.
3. **Deep internal parser/resolver module**: Knowledge owns deterministic Markdown extraction and physical path catalog resolution. One Admin-only KnowledgeCore.graph() read interface provides global/local nodes, edges and diagnostics; the HTTP adapter does not duplicate resolving logic. The Web link picker reuses the existing Wiki physical tree.
4. **Physical Vault contract**: Obsidian opens the knowledge/wiki/ directory, not the wider knowledge/ root. Web generated links use explicit Vault-relative paths without the .md suffix so Obsidian resolves them even when different folders contain identical basenames. Obsidian's root-level `.obsidian/` configuration directory is ignored by the Wiki scanner and Explorer (after symlink rejection); this is not a general ignore rule for arbitrary hidden content. Broken/ambiguous references are visible as diagnostics and are not considered valid edges.
5. **Preserve publication trust**: verified canonical Wiki is indexed, staged replacement drafts never replace accepted graph content before publish; initial canonical drafts may be visible to authenticated Admin with explicit draft styling, but never become Agent-readable through a graph tool.
6. **Protect moves**: fail closed on a move when prospective physical paths would break or retarget currently resolved links; return bounded diagnostics. Rewriting other published pages would require their own revision CAS and provenance-verified publish; it must not be done implicitly.
7. **Viewer-only Obsidian interoperability now**: reading the Vault works; external writable editing and syncing is not implied or silently enabled. Future writable Obsidian sync needs a separate integrity/CAS decision.
8. **Web view, not a second knowledge store**: extend existing /knowledge Web UI with Global/Local Graph, Backlinks/Outgoing, and a link-insertion helper that writes Markdown through current edit/publish flow. Existing physical file browser remains authoritative.

## Consequences

- Positive: native Obsidian Graph without bespoke import/export, graph rebuild after index loss, one source of truth, no change to main DB/Memory/Agent Job.
- Negative: file-based resolution requires correct physical path handling; move may be blocked until link edits are published. A full-corpus graph resolve during reconciliation adds bounded work; optimize only with benchmark evidence.
- Correctness guard: an unhealthy or partially scanned vault cannot return a purportedly current graph. One transaction publishes graph and FTS projection state together.
- Deferred: semantic/AI inferred links, Obsidian plugin-specific resolution, direct graph editing, automated backlink rewrites, cross-vault references, filesystem watch and Obsidian write sync.

## Alternatives rejected

- SQL-only graph: would be invisible to Obsidian.
- Stable-ID Wikilinks: current file names are not stable IDs and Obsidian does not use our custom YAML id for targeting.
- Separate graph database, authored YAML link catalog, virtual folders: duplicate authority and lifecycle complexity.
- Silent rewrite of existing canonical backlinks on move: bypasses Source verification and optimistic publication revision policy.

## Verification gate

Round-trip a Web-created [[AI/LLM]] reference to a real Obsidian Vault and verify that Global/Local Graph and Backlinks in both systems show the same source -> target pair. Exercise ambiguity, drafts, provenance checks, graph rebuild, stale-index fail-close and blocked unsafe move. No feature is marked implemented until these tests pass.
