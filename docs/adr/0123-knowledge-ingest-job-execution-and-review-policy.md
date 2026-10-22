# ADR 0123 — Knowledge Processing owns publication policy

**Status:** Accepted design; implementation pending.  
**Date:** 2026-10-19.  
**Target branch:** `feat/knowledge-memory-v3-file-first-index`.  
**Implementation specification:** [AI Ingest, Review, Auto Publish and Provider Settings](../knowledge/ai-ingest-auto-publish-implementation-v3.md).

## Context

Knowledge V3 already owns immutable Source snapshots, Drafts, Wiki Markdown, Source verification, revision CAS and the verified-only search index. Agent Job V3 already owns background execution, provider pins, model usage, retries and events. The feature should turn immutable Sources into evidence-backed Wiki while escalating conflicts and unsupported claims to a human review queue.

Production `submit_job` currently hardcodes `hierarchical_v3` with global Orchestrator/Worker pins; plugin-operation Jobs have no LLM provider. Both are inappropriate for single-Source AI analysis without deliberate extension.

## Decision

1. **KnowledgeProcessing** is a deep module owning source qualification, candidate admission, publication policy, Review decisions and reconciliation. Its interface is narrow: analyze, decide, list reviews.
2. **Agent Job V3** remains the only scheduler and execution ledger. Introduce `knowledge_ingest_v3` as an explicit bounded single-LLM Structured Output mode. No new job scheduler, plugin hack or Root/Child rewrite.
3. **KnowledgeCore** is the only trusted Markdown/Source writer. LLM output is an untrusted proposal, never a direct filesystem command; maintain Draft CAS, Source hash validation, accepted revisions and index recovery.
4. **KnowledgePolicyGate** (private) classifies into Publish, Review or Reject. Missing source quotes, contradiction, ambiguous duplicate or update, partial Source coverage, stale search and uncertain inference go to Review. Source byte verification is not semantic truth verification.
5. **Provider selection** belongs in Knowledge Settings, referencing the existing LLM Provider catalog. Per-job immutable provider pin and policy/source snapshots must not overwrite global Agent Job or Voice Agent/Template provider bindings.
6. **Persistence** adds small settings and proposal-decision metadata tables in the existing application SQLite. Canonical Markdown remains file-first; no fourth SQLite file, separate ingest-run ledger or second queue.
7. **Rollout** starts with `review_all` and manual Analyze. `auto_publish` is opt-in after real strict-structured adapter tests, failure-injection tests and review-policy validation. Initially auto-publish only directly evidenced new pages, not destructive page merges.
8. **Web** preserves the real Raw/Wiki/Drafts explorer, adding independent Review and Settings panes. Reuse existing Job Timeline for LLM execution status.

## Rejected alternatives

- **All knowledge logic inside generic Root/Sub-Agent:** conflates publishing authority with model-led orchestration and adds unnecessary steps for one Source.
- **Standalone Knowledge jobs/scheduler:** duplicates Job V3 state, pinning, cancellation, retry and activity.
- **Plugin-operation Job with fake LLM behavior:** plugin-only runs intentionally do not have an LLM execution contract.
- **Direct AI write to Wiki:** bypasses Rust verification, draft history and CAS.
- **Knowledge-specific LLM provider registry or credentials:** duplicates the shared Provider Runtime and existing security decisions.
- **Mandatory independent AI verifier in V1:** extra cost does not establish truth.


## Reconciliation with committed Knowledge UI and ADR 0124 (2026-10-19)

**Audited HEAD:** `1d3f80282288f33417094c91660338c3fb176033`; details: [post-spec branch audit](../knowledge/ai-ingest-branch-reconciliation-2026-10-19.md). This reconciliation does **not** change ownership or claim automatic ingest is implemented.

- [ADR 0124](0124-wiki-wikilinks-graph-projection.md) is now the authoritative Wiki relation contract: the Markdown body in `knowledge/wiki/**/*.md` owns real Wikilinks; `kb_links` (Knowledge index migration `0003` / indexer v3) is a rebuildable derivative alongside FTS/chunks. KnowledgeProcessing **must not** invent another canonical graph relation store, silently add inferred relationships, or rewrite canonical backlinks without CAS/source verification.
- `KnowledgePolicyGate` now also evaluates proposed Wikilink targets through the existing Knowledge resolver and current **verified** Wiki catalog. Broken/ambiguous links, references to draft-only Wiki pages, path-unsafe moves or cross-candidate unverified targets route to Review in initial auto-publish mode. Admin Graph may include draft nodes, so Graph appearance **never** grants publish authority.
- The Obsidian Vault root is `knowledge/wiki/`, and generated links must be physical Vault-relative names (e.g. `[[AI/LLM]]`), not stable-ID links. Graph index stale/unavailable fails closed; canonical publication remains durable when FTS/`kb_links` reconciliation fails.
- Knowledge Web now has a physical Raw/Wiki/Drafts selector, Reading/Compose/Graph modes, inline Markdown authoring and an explicit dirty-editor guard. Review is a **logical** queue, not a new physical root; it may not bypass or overwrite a human Compose draft.
- `/knowledge/settings` and `KnowledgeSettingsPage.vue` **already exist** as an example-only page (no saved settings, no provider input and no endpoint). Implement the Knowledge provider selector there, preserving the vault-footer gear and existing back-navigation. Do not add a redundant Settings tab to the physical Explorer.
- The present production Agent Job mode still needs explicit `knowledge_ingest_v3` admission/scheduler/pin changes. The new Graph UI and Settings placeholder do not implement AI ingest, automated publication or a Review Queue.

## Consequences and invariants

The new Job mode requires explicit production changes to admission, snapshot handling, eligibility, dispatch and traced single-provider turns. Preserve all historical Job modes. Current Source v1 accepts up to 256 KiB but a model turn is bounded to 65,536 input bytes; V1 must fail closed for oversized one-turn Sources, not silently truncate and publish.

Publication may commit canonical Markdown before application SQLite metadata or FTS reconciliation completes. Persist publish intent and reconcile idempotently from revision, Source ref and content hash; ambiguous cases require Admin intervention. Never erase a committed revision due to a later DB/index failure.

Existing `knowledge.search` and `knowledge.read` remain verified-only; Memory, Agent Job global provider choices and Physical Explorer are not modified.

**This ADR records the implementation choice, not shipped functionality.**
