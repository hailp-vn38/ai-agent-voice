# Knowledge V3 — AI Ingest, Review Queue, Auto Publish and Provider Settings

**Status:** Implementation and deterministic local verification complete; deployment acceptance is not claimed.

**Implementation scope (2026-10-19):** Backend P0–P3 and the existing Knowledge Settings UI are implemented. The user explicitly limited UI changes to Settings; the Review workspace and Source Analyze button remain outside this UI change. See [implementation/verification record](ai-ingest-verification-v3.md) for API contracts, qualified adapter scope and remaining operational boundaries. The pre-implementation observations below are retained as the design baseline.
**Target branch:** `feat/knowledge-memory-v3-file-first-index` (based on `feat/agent-job-v3-opencode-session-runtime`).
**Date:** 2026-10-19.
**Decision:** Knowledge owns ingestion/publication policy; reuse Agent Job V3's scheduler, ledger, provider admission and LLM execution. No standalone knowledge scheduler or second ingest-run state machine.

**Branch alignment (2026-10-19):** Revalidated against `1d3f80282288f33417094c91660338c3fb176033` after 145 commits since the document-index baseline. See [branch change audit](ai-ingest-branch-reconciliation-2026-10-19.md) and [ADR 0124](../adr/0124-wiki-wikilinks-graph-projection.md). The Graph implementation and example-only `/knowledge/settings` route are committed; AI Ingest, stored settings, Review Queue and Auto Publish remain **proposed, not implemented**.

## 1. Goal and non-goals

Implement **Source → AI analysis → trusted Rust validation → Auto Publish**, with a **Review Queue** for contradictory, unsupported, ambiguous or outdated proposals. Users configure a Knowledge-specific LLM provider in the existing Knowledge workspace.

Acceptance is strictly opt-in: the initial configuration uses `review_all`, and `auto_publish` becomes selectable only after provider qualification and validation gates pass. Wiki remains file-first; Memory stays a different domain.

Non-goals for this slice:
- A new generic autonomous agent, second scheduler, plugin installation, arbitrary LLM filesystem writing or new Knowledge index database.
- Replacing Root/Child OpenCode-style Agent Job V3, the global Agent Job provider choices or the user-approved physical Raw/Wiki/Drafts tree.
- Full binary/PDF/vision/audio extraction, web crawling, vector matching, Git sync, multi-owner approval or automatic Memory writing.
- Claiming that Rust can prove semantic truth purely from citations. Ambiguous interpretation is always routed to review.

## 2. Existing implementation to preserve

Verified on the target branch:
- `KnowledgeCore` in `crates/voice-agent-server/src/services/knowledge/` owns immutable Markdown Sources, `put_draft_at`, `publish(id, expected_revision, source_ref)`, canonical `verified` revisions, revision CAS, filesystem locking, provenance SHA checks and SearchIndex reconciliation.
- New Wiki drafts may be stored in `knowledge/wiki/` with `status: draft`; updates to previously verified Wiki use `knowledge/drafts/`. Do **not** assume the Review Queue is identical to the physical `drafts/` folder.
- Source v1 is bounded UTF-8 Markdown (`MAX_SOURCE_BYTES = 256 KiB`) with immutable `src_<id>:1` reference. Editable Raw files are **not** automatically verified Source snapshots; `raw_import` is explicit.
- Agent `knowledge.search` and `knowledge.read` expose only verified Wiki with trustworthy Source provenance. `core.memory` and Memory policies are unchanged.
- Agent Job V3 persists `jobs`, Job/LLM events and immutable provider pins; `JobRequest` currently has `goal`/`output_format`/`required_sources`/`required_capabilities`. Production `submit_job` hardcodes `hierarchical_v3` with **two global provider pins**. The scheduler sends non-plugin jobs through Root/Child orchestration. An alternative execution mode is **new work**, not a configuration switch.
- `JobModels::turn_traced` already supports `BackgroundMode::Structured { name, schema }`. `ManagedModels` performs provider admission/capacity, authorization, cancellation, tracing and exact pin validation. LLM request messages are currently capped at 65,536 bytes. `supports_structured_outputs` is provider configuration, not proof of a qualified runtime.
- Existing app SQLite owns Job state, Providers and Plugins; `knowledge_index.db` and `memory_index.db` are independently rebuildable projections. No fourth database.
- Existing `/knowledge` Vue page contains physical-space Raw/Wiki/Drafts explorer. Preserve it and add Review and Settings **as workspaces, not fake folders**.

Normative predecessors: `docs/knowledge/implementation-v3.md`, `docs/knowledge/database-layout-v3.md`, `docs/knowledge/physical-folders-implementation-v3.md`, `docs/flows/13-knowledge-v3.md` and ADRs 0117–0122.


### 2.1 Implementation deltas since this specification was drafted

**Actual current surfaces, not future placeholders:**

- `/knowledge` now contains a physical Raw/Wiki/Drafts explorer (root selection persists), a Reading/Compose/Graph workspace, an Obsidian-inspired inline Markdown note creation flow, safe read-only Markdown/Mermaid rendering and the existing Source/draft publication contract. It must **not** be redesigned as a five-root Raw/Wiki/Drafts/Review/Settings file tree.
- `/knowledge/settings` is an **existing separate Vue route** in `KnowledgeSettingsPage.vue`, launched from the gear in the vault footer. It renders informational Vault / Index & Graph / Publishing examples. It contains **no inputs, selects, form or persistence** today. Upgrade this page to host real AI/Provider settings, keeping its route and back-navigation.
- Existing `KnowledgeGraph.vue` and `GET /api/admin/knowledge/graph` present Global/Local Graph, outgoing/incoming links and unresolved diagnostics using `cytoscape`. `wiki_links.rs` extracts/resolves `[[Wikilinks]]`, embeds and Markdown note links; `KnowledgeCore::move_page` blocks changes that would break existing resolved links.
- `migrations/knowledge/0003_wikilink_graph.sql` adds rebuildable `kb_links` to the **existing** `knowledge_index.db`; Knowledge indexer is **version 3**. Its reconcile transaction refreshes FTS, passages and graph links from canonical Wiki Markdown. No graph-authority database is added.
- Graph is **Admin oriented** and may show initial `wiki/` pages with `status: draft`; staged replacement drafts in `drafts/` do not overwrite accepted graph content. **Graph presence does not imply Agent-visible verified knowledge.** Source-verified `knowledge.search`/`knowledge.read` continue to exclude drafts.
- No code inspected at the audit HEAD implements `knowledge_ingest_v3`, Knowledge processing settings, AI candidate validation/Review Queue or automated publish. Existing production Job submission still hardcodes the standard mode and provider pair.
- Existing Background Settings navigation has moved into the Jobs page; Knowledge provider settings must remain independent from that global configuration.

**Documents to preserve:** [ADR 0124](../adr/0124-wiki-wikilinks-graph-projection.md), [Graph implementation](wiki-graph-view-implementation-v3.md), [Graph flow 14](../flows/14-knowledge-graph-v3.md), [Obsidian smoke](obsidian-graph-smoke-v3.md), [Physical Explorer](physical-folders-implementation-v3.md). This plan extends them; it does not supersede any of them.

## 3. Ownership and deep modules

| Module | Small Interface (proposed) | Hidden Implementation and invariant |
|---|---|---|
| `KnowledgeProcessing` | `analyze(source_ref, options, idempotency_key) -> job_id`; `decide(proposal_id, decision, expected_revision) -> Outcome`; `list_reviews(filter)` | Owns the policy and proposal lifecycle, dedup, review admission, publish reconciliation, and knowledge-specific input validation. Delegates model execution and trusted writes. |
| `KnowledgeCore` (existing, extend narrowly) | `stage_proposal(..)` or reuse `put_draft_at(..)`; `publish(..)`; verified reads | Exclusive trusted canonical Wiki/Source writer; locks, revision CAS, immutable Source validation, index reconciliation. |
| `Agent Job V3` (existing, extend) | `submit_knowledge_ingest(snapshot, idempotency)`; internal `execute_knowledge_ingest(job_id)` | Sole owner of queued/running/terminal execution, cancellation, deadlines, retry/recovery, model turns, usage and timeline. |
| `KnowledgePolicyGate` (private to KnowledgeProcessing) | `assess(candidate, source, target, policy) -> Publish \| Review(reasons) \| Reject(error)` | Deterministic, testable decision: source checksum, quote spans, title/path, candidate collisions, target base revision, syntax and boundedness. Returns data; no writes. |
| `Provider Adapter` (existing) | `turn_traced(pin, request, Structured, deadline, cancellation)` | Shared runtime/pins/authorization; rejects unsupported structured output. No Knowledge-specific LLM provider implementation. |
| `Admin Knowledge Adapter` | Authenticated Settings / Analyze / Reviews / Approve / Reject HTTP interfaces | Input/output mapping only; no independent SQL or filesystem mutations. |

**Seams:** (1) KnowledgeProcessing → Agent Job execution; (2) KnowledgeProcessing → KnowledgeCore publication; (3) Agent Job → existing LLM provider adapter. Keep validation behind a **deep** KnowledgePolicyGate interface. The deletion test: removing KnowledgeProcessing should force every caller to duplicate source qualification, review policy, dedup and recovery.

Do not add an abstract “provider strategy”, a second queue engine or interchangeable publication adapters before a second real implementation needs such a seam.

## 4. End-to-end processing

~~~mermaid
flowchart TD
  A["Admin imports immutable Source (or explicitly imports Raw)"] --> B["KnowledgeProcessing.analyze"]
  B --> C["Validate Source version/hash and Knowledge settings"]
  C --> D["Agent Job V3: knowledge_ingest_v3 with pinned provider"]
  D --> E["Strict structured LLM candidate(s)"]
  E --> F["KnowledgePolicyGate (Rust, no writes)"]
  F -->|Eligible| G["Persist proposal intent; stage via KnowledgeCore"]
  F -->|Uncertain| H["Persist proposal intent; stage pending Review"]
  F -->|Invalid output| X["Bounded repair; else terminal Job failure"]
  G --> I["KnowledgeCore.publish with Source verify + CAS"]
  I --> J["Verified Wiki; reconcile FTS"]
  H --> K["Admin Review: approve / reject / edit"]
  K -->|Approve; CAS + recheck| I
  K -->|Reject| L["Close proposal; leave published Wiki untouched"]
  J --> M["Finalize proposal outcome + Job event"]
  H --> M
~~~

### 4.1 Admission and source selection

- Trigger initially: explicit `Analyze Source` action for a `src_<id>:1` immutable snapshot. Optional `auto_after_import` triggers **only after immutable import commits**, not every Raw edit or file watcher event.
- Verify Source ID, version, SHA256 and presence of extracted Markdown **before** submit; pin `source_ref` and SHA256 in Job snapshot. Re-verify before any publish.
- One Job processes **one Source version**, returning 0–5 bounded Wiki candidates; one Source can create multiple Wiki pages. Multiple Source merging and cross-source citation publication are deferred.
- Reject empty/oversized/unavailable Source. Current `turn_traced` has a 65,536-byte total message limit. Phase 1 processes only Source text that fits a bounded ~40 KiB content budget after prompts/schema; **never truncate a long Source and then auto-publish an incomplete interpretation**. Oversize: typed `source_too_large_for_analysis`, instruct import/chunk support. Later implement deterministic line-aware multi-pass inside **the same Job**, with source-span carry-forward and coverage check.
- Idempotency fingerprint includes source ID/version/hash, policy revision and exact provider pin. Repeating a lost-ACK submit with the same key returns the original Job; same key with different inputs is a conflict. Separate explicit `Re-analyze` creates a new run.
- Prevent simultaneous active analyzes for the same immutable Source under the same policy revision unless explicitly requested as a new run; no duplicate publication for the same proposal.

### 4.2 One-LLM execution mode, not a Root/Child rewrite

Add `knowledge_ingest_v3` as an explicit production mode. It **does not** issue a Root/Child delegation or model-authored arbitrary tool calls.

Existing `submit_job` always pins global Orchestrator+Worker and sets `hierarchical_v3`. Therefore implement a **mode-aware admission path**, with a shared private durable Job insertion helper where practical:
- Keep the existing `JobRequest` JSON shape for all historical job readers; construct a server-generated display goal. Do **not** encode Source bytes or trusted refs as a free-form model goal. The dedicated `KnowledgeIngestSnapshot` in `JobSnapshot` stores Source ref/hash, policy revision, selected provider pin and processing limits. All optional new snapshot fields deserialize old jobs.
- Use the existing `ProviderPin` representation for the **single** analysis provider; the unused Orchestrator pin must not be validated for `knowledge_ingest_v3`. An explicit tagged mode determines the required pin set; existing modes retain their old gates.
- Branch existing scheduler/eligibility/executor by exact execution mode: `plugin_operation_v3` unchanged, `hierarchical_v3` unchanged, `knowledge_ingest_v3` executes dedicated bounded LLM worker. Do not hijack plugin-operation, skip existing provider reservations, or apply the Agent Job global provider pair.
- Knowledge enabled flag is separate from the Agent Job Orchestrator toggle; the shared runtime's master shutdown, admission authorization, provider revocation, limits, cancellation and worker semaphore remain authoritative. Disabled Knowledge prevents **new** intake and execution; pending jobs are cancelled or deterministically rejected by the eligibility gate.
- Create the minimum durable Agent Run / LLM activity linkage needed for existing usage/timeline; no independent Knowledge run-state table.
- One `BackgroundMode::Structured` turn with frozen schema; on invalid JSON/schema, at most **one** bounded repair attempt using the same pinned model. No tool-call fallback and **no** conversion of free-form text to auto-publish.
- A Job can complete with outcomes `auto_published`, `pending_review` and `skipped` per candidate. `pending_review` is **not** a running Job status.

### 4.3 Candidate output contract

Strict provider Structured Output, JSON Schema with `additionalProperties: false`, fixed bounds; Rust separately parses with `deny_unknown_fields` and validates all fields.

~~~json
{
  "candidates": [
    {
      "action": "create",
      "target_wiki_id": null,
      "title": "Retrieval Augmented Generation",
      "markdown": "# RAG\n...",
      "suggested_folder": "wiki/AI",
      "claims": [
        {
          "text": "RAG retrieves documents before generation.",
          "source_ref": "src_example:1",
          "from_line": 10,
          "to_line": 12,
          "evidence_quote": "Retrieval precedes generation..."
        }
      ],
      "possible_duplicates": [],
      "conflicts": [],
      "unresolved": []
    }
  ]
}
~~~

- V1 `action = create | update`; update needs a verified `target_wiki_id` and pinned `expected_revision` from read context. Use trusted server search over verified Wiki to offer candidate matches; never trust AI-supplied IDs or folder paths without resolution and validation.
- Only server-imported Source ref/version is accepted for V1 candidate claims. Reject foreign, missing or stale refs; do not allow tool-generated filenames or arbitrary paths.
- Each claim's quoted text must be an exact bounded snippet inside the specified source line range. A quoted match proves textual grounding, **not** that a paraphrased claim is semantically entailed. Unsupported inference, weak paraphrase, unquoted claims, disagreement or low coverage ⇒ Review; NEVER convert model confidence scores into permission to auto-publish.
- One candidate needs a non-empty Markdown body and at least one valid claim/source citation. Extract explicit unresolved/contradiction arrays for the Review reasons; also conduct independent server-side checks.
- LLM cannot choose `status=verified`, `review_status`, provider, source version, policy or published revision.

### 4.4 Deterministic publish gate

The `KnowledgePolicyGate` **returns one of**:
- `Publish` only when auto is enabled **and** all positive invariants hold: valid immutable Source checksum; all claim source spans/quotes verified; coverage within analyzed Source; no unresolved or conflicts; exact target identity; allowed Markdown/path; current verified Wiki index healthy for duplicate search; no duplicate/ambiguous candidate; target revision matches; proposal has not already been applied.
- `Review(reason_codes)` for missing/insufficient support, conflicting content, plausible duplicate, ambiguous update, stale retrieval, unqualified semantic transformation, incomplete Source, incompatible target revision, user-modified draft or policy requiring review.
- `Reject(error_code)` for malformed schema, invalid Source ID, secret/path traversal, oversized payload or other technically inadmissible proposal. Provider failure is a Job failure, not a human-review item masquerading as valid knowledge.

**Graph/Wikilink integrity gate (post-ADR 0124):** inspect the candidate Markdown through the existing `services/knowledge/wiki_links.rs` extraction/resolution seam, using a verified canonical Wiki path catalog. Generated targets must use Obsidian-compatible Vault-relative `[[folder/Note]]` paths (no `.md` extension), not stable Wiki IDs or new SQL relationships. Route missing/ambiguous links, references to unverified initial Drafts, and links that would require renaming/moving another page to **Review**. Do not auto-publish simultaneous candidate-to-candidate links in V1: another candidate's not-yet-verified page is not a valid proven target. A Wiki proposal may not silently mutate backlinks. A valid link also is not factual evidence for a claim; Source quote/provenance checks remain mandatory.

Initial auto-publish scope: **new pages grounded in direct source excerpts**, not arbitrary synthesis. Existing-page rewrites/merges should default to Review until deterministic non-destructive merge gates and tests exist. Publish of a proposed update must never overwrite a previously verified page silently.

Use `knowledge.search`/the existing Knowledge FTS5 and verified canonical reads for duplicate candidates; normalize title/aliases and check exact identity. If the index is stale or no reliable candidate window can be constructed, fail closed to Review. Heuristic semantic conflict detection may be added later, but must not be described as deterministic fact checking.

Review reason codes (stable initial set): `missing_evidence`, `source_quote_mismatch`, `source_incomplete`, `conflicting_claim`, `possible_duplicate`, `ambiguous_target`, `unsupported_inference`, `stale_source`, `revision_conflict`, `index_stale`, `policy_review_all`, `validation_uncertain`.

### 4.5 Publication and crash recovery

Keep `KnowledgeCore` the **only** canonical writer. It already distinguishes a fresh draft from a staged replacement, verifies Source at `publish`, commits accepted revisions atomically, and reports a committed publish successfully even if FTS reconciliation degrades.

- Persist a deterministic proposal identity `(job_id, candidate_index)` and source/policy/expected target revision **before staging**. Allocate and persist stable Wiki ID once. Proposed Markdown body lives in Knowledge draft Markdown, **not duplicated in the SQL queue**; store only SHA256/size and pointers in the main DB.
- Within the canonical write lock, stage through existing draft interface and re-evaluate gate on the exact staged revision before auto-publish/approve. A reviewed edit changes draft revision and invalidates prior gate; revalidate with current evidence and mark review again when needed.
- Use states such as `staging`, `pending_review`, `publishing`, `published`, `rejected`, `needs_reconcile` for **proposal decisions only**. Job status remains owned by Agent Job V3; no running-review Job.
- Before filesystem commit, persist a `publishing` intent with expected target revision, immutable Source ref and proposed content hash. On restart, reconcile intent with canonical revision/history, source reference and content hash; matching accepted content ⇒ mark `published` without reapplying. Ambiguous state ⇒ `needs_reconcile` and Admin intervention, never blind republish.
- After successful canonical commit, finalize proposal metadata and append Job event. DB and filesystem are not atomic together; never claim rollback of published Markdown if app SQLite write fails. Reconciliation is idempotent and monotonic.
- If index update fails after canonical publish, preserve `published`, mark index unhealthy and make **search and Admin Graph** refuse stale results. The current v3 Knowledge index reconciliation also updates `kb_links`; link projection is rebuildable and never a separate canonical writer. A degraded index is **not** a reason to republish/duplicate.
- Review decision requires expected proposal revision plus staged Wiki revision and, when present, verified canonical base revision. Retry/duplicate Approve is read-only if already accepted; changed draft/source/target is a conflict, not a second publish.
- When a new draft is rejected, keep its history if policy requires audit; do not expose it to Agent Knowledge. Removal/archival can be a separate explicit Admin action.

## 5. Persistence layout

**Canonical Markdown, unchanged roots:**

~~~text
data/
  knowledge/
    raw/                  # editable Raw + immutable src_<id>/1 snapshots
    wiki/                 # canonical Wiki: draft for new; verified for accepted
    drafts/               # replacements staged against already verified Wiki
    revisions/
    draft_revisions/
    knowledge_index.db    # rebuildable FTS5/BM25, passages + kb_links (indexer v3)
  memory/                 # unchanged
    facts/
    MEMORY.md
    memory_index.db
~~~

**Application SQLite additions (next free migration; 0123 if still available):**
- `knowledge_processing_settings`: singleton revision, `enabled` (default false during rollout), `analysis_provider_ref`, `publication_policy` (`review_all | auto_publish | draft_only`; default review_all), `trigger_mode` (`manual | auto_after_import`; default manual), limits and optional future verification config.
- `knowledge_proposals`: stable proposal ID, `job_id`, candidate index, Source ref/hash, Wiki ID, candidate/draft SHA256, draft revision, pinned canonical-base revision, policy revision, state, bounded reason codes, actor/decision time, accepted revision and timestamps. Unique `(job_id, candidate_index)` and controlled single pending/publishing proposal per target Wiki. Store metadata only; no duplicate canonical Markdown content.
- Job snapshot and existing Job Events hold execution pins/usage/result details; do **not** add `knowledge_ingest_runs` or a fourth SQLite file.

Application DB is durable operational metadata; file-first Markdown stays authoritative for Wiki content. Proposal state reconciles from Markdown when possible and never turns a failed/missing vault into an empty knowledge corpus.

## 6. Provider selection and settings

The Provider catalog (`/api/admin/providers`) remains the single place to create/test/manage providers. Knowledge Settings only selects an existing **LLM provider ref**, no API keys/secrets stored there.

Proposed settings:

~~~json
{
  "enabled": false,
  "analysis_provider_ref": null,
  "publication_policy": "review_all",
  "trigger_mode": "manual",
  "max_candidates_per_source": 5,
  "verification": { "enabled": false, "provider_ref": null },
  "revision": 1
}
~~~

- `GET /api/admin/knowledge/settings` returns settings, revision, readiness and **eligible LLM options** filtered from the existing provider/runtime registry. Each option gives ref, display name, runtime readiness and structured-output qualification status.
- `PUT /api/admin/knowledge/settings` uses expected revision/CAS. Reject enabling if no selected qualified structured provider, or policy `auto_publish` when capability qualification is absent.
- At Job admission, resolve `analysis_provider_ref` via existing provider pin infrastructure and snapshot incarnation/revision/digest. Re-check runtime capability and authorization before the LLM turn. Missing/deleted/disabled/rotated provider fails closed, not silently falls back to another.
- Existing global `orchestrator_llm_provider` / `worker_llm_provider` are **not** overwritten. Voice Agent/Template providers, Provider Test, and Root/Child pin semantics are untouched.
- Existing configuration has `supports_responses` / `supports_structured_outputs` hints for supported LLM adapters. A declared flag is **not sufficient**; actual adapter path must pass strict structured qualification tests (valid schema, refusal, truncation, unsupported mode, timeout). Provider option must be hidden/disabled for auto until qualified.
- Optional independent verifier is **Phase 4**. If enabled later, pin its provider separately; no assumption that a second LLM independently proves factual truth.
- Settings change applies to **new jobs**. Running jobs retain the provider/policy/Source snapshot; a later re-analysis explicitly takes new settings. Revocation still blocks the old pin at the existing authorization seam.

## 7. Admin HTTP interfaces

Add under existing authenticated Knowledge admin adapter, preserve all existing routes:

| Method | Route | Behavior |
|---|---|---|
| GET | `/api/admin/knowledge/settings` | Settings/revision, provider options and readiness. |
| PUT | `/api/admin/knowledge/settings` | CAS update, validated provider/capability/policy. |
| POST | `/api/admin/knowledge/sources/{id}/analyze` | Admin-only submit, pinned source version; `Idempotency-Key` supported, returns Job link. |
| GET | `/api/admin/knowledge/reviews` | Filter/paginate review metadata, include proposal/job/wiki refs and reasons. |
| GET | `/api/admin/knowledge/reviews/{id}` | Exact draft/read/provenance for side-by-side review, never arbitrary path. |
| POST | `/api/admin/knowledge/reviews/{id}/approve` | CAS, reverify source/target and gate, publish at most once. |
| POST | `/api/admin/knowledge/reviews/{id}/reject` | Record decision; leave current verified Wiki untouched. |

Use existing `/api/admin/knowledge/pages` and `/publish` for ordinary **human authored** revisions. The Review actions must not bypass those KnowledgeCore invariants.

Representative typed responses: `202 {job_id, source_ref, replayed}` for analysis; `409 revision_conflict` on changed proposal/target; `422 source_evidence_unverified` on invalid support; `503 provider_unavailable` for unready selected provider. Existing Admin auth remains authoritative.

## 8. Web design — integrate with the committed Knowledge workspace

**Preserve exactly the current UI hierarchy:**

- `/knowledge`: the physical root selector continues to offer **Raw | Wiki | Drafts** only. Current workspace modes are **Reading | Compose | Graph**; `KnowledgeGraph.vue` and existing Markdown reader/editor remain intact.
- `/knowledge/settings`: a **preexisting separate route**, launched by the settings gear in the left vault footer. Replace **example-only** cards in `KnowledgeSettingsPage.vue` with real Knowledge AI Processing/Provider controls **without changing navigation**. Reuse its back-to-Knowledge action, layout, i18n, and present Vault / Index & Graph / Publishing areas as appropriate.
- **Review Queue**: a **logical view** attached to Knowledge workspace (e.g. a Review work mode/tab or side panel), **not** a fourth physical root. Its contents come from app-DB `knowledge_proposals` metadata and existing Wiki Drafts. It must not hide or replace the current reader/graph, or require a new queue service.
- **Raw / Sources:** immutable Source bundles stay read-only in the explorer; only an explicitly imported immutable `src_<id>:1` can be analyzed. Add `Analyze` to the committed Source detail/action with the current provider displayed; saving an editable Raw note alone must not submit AI work.
- **Compose and human edits:** preserve the current inline filename creation, full-height Markdown editor, explicit save, Ctrl/Cmd+S, dirty-state guard, canonical Wiki revision CAS and Raw SHA-256 CAS. AI analysis/Review must **not** publish unsaved browser editor text or replace a human-staged Wiki draft.
- **Wiki proposals:** show side-by-side Source line evidence, current verified Wiki revision/diff (for update) and a **read-only proposed Markdown preview** using the existing safe Markdown renderer. On edits, revalidate and preserve expected draft revision.
- **Graph-aware proposal:** validate generated `[[Vault/Relative/Note]]` targets before deciding publish; Graph Link details and unresolved diagnostics may help the reviewer, but do **not** make Admin Graph nodes authoritative. A candidate that links to an unverified page remains Review-only.
- **Processing visibility:** link proposal to existing Job V3 Detail Timeline. Existing Background Settings live under Jobs and must not be relocated or overwritten.
- **Memory:** preserve `/memory` and its policy. Every new label needs vi/en i18n; use existing tested Vue conventions.

Existing regression tests include `KnowledgePage.test.ts`, `KnowledgeGraph.test.ts`, `KnowledgeSettingsPage.test.ts`, `knowledge-settings-routes.test.ts`, Markdown renderer tests, and the branch's Graph Rust tests. Keep these passing while adding real settings and review tests.

## 9. Error, retry and security matrix

| Condition | Durable behavior |
|---|---|
| Provider not capable of strict schema | Reject admission or fail typed; no text fallback. |
| Invalid structured output | One bounded repair, then terminal Job failure; no Wiki mutation. |
| Source SHA/provenance mismatch | Fail closed, no publish; existing verified data stays intact. |
| Source above single-turn limit | Typed oversize; no silent truncation. |
| Missing claim quote / semantic inference | Draft + `pending_review`. |
| Likely duplicate or conflicting claim | Draft + `pending_review`, reason recorded. |
| Target revision changed since proposal | 409/review; never silently overwrite. |
| Existing draft already staged by human | Do not overwrite; Review conflict. |
| Cancel during LLM | Job cancellation; no canonical mutation. |
| Cancel after durable publish intent | Reconcile before finishing; never attempt rollback via deletion. |
| Restart after Markdown commit, before DB ACK | Compare immutable revision/content/source and settle proposal once; uncertain ⇒ `needs_reconcile`. |
| FTS5 reconcile failed | Keep canonical published revision; mark search stale and repair index later. |
| Provider disabled/revised mid-flight | Existing pin eligibility/authorization rejects; never select alternative silently. |

Guard user-supplied folders under `wiki/`, reject symlinks/traversal/duplicate stable IDs, secrets and untrusted model paths. Approval is Admin-only. Limit file size, candidate count, time, token budget and concurrent processing via Job V3 controls. Structured provider execution never grants general write tools.

## 10. Implementation map and phases

### P0 — Contract and admission (no auto-publish)
- Add proposed settings migration and types; typed `KnowledgeIngestSnapshot` / new mode.
- Extend mode-aware Job admission, dispatcher/eligibility, single-provider pin and trace path; do not change existing hierarchical/plugin modes.
- Provider readiness/structured qualification; explicit Analyze endpoint; provider persistence CAS.
- Parse bounded candidate schema; run with fake provider; return durable Job result only.
- Acceptance: normal Agent Job V3 + plugin-operation tests remain green; Knowledge works with its provider even when global Orchestrator/Worker values differ.

### P1 — Deterministic gate, Wiki-link resolution and draft/review
- Build private `KnowledgePolicyGate` with immutable-source quote verification, exact target revision, dedup/FTS+graph readiness, existing Wiki link resolution, target verified-status checks and stable review reasons.
- Persist `knowledge_proposals`; stage through `KnowledgeCore`; deliver Review listing/detail.
- Handle new draft in `wiki/` and replacement draft in `drafts/`; preserve physical folder conventions and Source-only scope.
- Acceptance: unsupported/conflicting/stale candidates never become `verified`; Agent search cannot read them.

### P2 — Safe auto-publish and recovery
- Enable policy `auto_publish` for qualifying **new directly grounded pages**; existing-page updates default Review until separate evidence.
- Add publish-intent reconciliation and idempotent approval; crash-injection tests between DB intent, staging, canonical commit, index and Job ACK.
- Acceptance: restart/retry cannot duplicate revisions, overwrite a human draft or lose evidence; index failure does not roll back or republish canonical Wiki.

### P3 — Admin UX and automated import trigger
- Add a **logical Review workspace** and Source Analyze action to `KnowledgePage.vue`; implement the real settings UI in the **existing** `KnowledgeSettingsPage.vue` at `/knowledge/settings`. Link Job Detail Timeline and source comparison; preserve Reading/Compose/Graph.
- Support `auto_after_import` only on committed immutable import. Preserve manual trigger as default.
- Acceptance: authenticated happy path and all review conditions operate through real endpoints; Raw/Wiki/Drafts root selection, Obsidian-style editor, Graph and Memory semantics remain unchanged.

### P4 — Explicitly deferred
- Optional second independent AI verifier; many-source consolidation; line-aware long-document passes; binary extraction; semantic similarity/contradiction models. Each requires separate qualification evidence, not a silent V1 dependency.

## 11. Test plan and release gates

**Unit:** structured schema fuzz/unknown fields, quote line offsets and Unicode, title normalization/aliases, source refs/sha, policy decision truth table, no-authority of LLM status/path, fixed provider pin, idempotency fingerprint, CAS.

**Integration:** actual Source import → knowledge Job → draft → review/auto publish → search, whole Job timeline/usage; disabled Knowledge; disabled/revised provider; invalid mode; existing verified Wiki update; human draft collision; multiple candidates; long source error; strict-schema provider rejection; FTS stale.

**Recovery:** fault injection at admission commit, proposal-intent commit, draft commit, Wiki canonical commit, search-index reconciliation and final Job ACK; restart and idempotent reconciliation assertions.

**Regression:** existing `hierarchical_v3` Root/Sub-Agent tool calls; `plugin_operation_v3`, core.knowledge read-only catalog, Agent/Template provider references, manual Admin Wiki publish, Physical Explorer A, Memory recall/forget and separate SQLite migrations.

Release gates:
1. No old Job API request, provider pin or plugin operation regression.
2. Strict Structured provider qualifications pass with real adapter fixtures; unqualified provider remains unavailable for Knowledge auto.
3. Every unverified/ambiguous proposal goes to Review; only verified canonical Wiki appears in Agent retrieval.
4. No duplicate publication after retries/restarts; SQLite/Markdown split-brain is detectable and repairable without content deletion.
5. Explicit configuration enables Auto Publish only after successful end-to-end validation.

## 12. Decisions / remaining constraints

**Approved architecture from the discussion:** separate Knowledge ownership, reuse Agent Job V3 execution, one analysis provider selected in Knowledge Settings, structured proposals, Rust publish gate, Review Queue for insufficient evidence/conflict, optional verifier later.

**Implementation constraints now made explicit:**
- `knowledge_ingest_v3` requires real changes to production Job admission/scheduler; current production code cannot run it via a settings toggle.
- The first source pipeline must obey the existing 65,536-byte model request cap and the 256 KiB Source vault cap.
- Default `review_all` and manual Analyze are deliberate rollout safeguards. `auto_publish` applies conservatively to directly supported new pages.
- A proposal queue is operational metadata in **existing application SQLite**, not a second scheduler/ingest-run ledger or canonical knowledge store.
- Reuse current `KnowledgeCore::publish` and existing vault/revision rules; any new helper belongs behind KnowledgeCore's interface, never in an independent writer.
- Preserve existing core policy read-only operations; automatic publication authority is held by trusted KnowledgeProcessing internals, **not** granted to arbitrary Root/Child tools.

## 13. Code ownership checklist

Proposed edit locations (not implemented by this document):
- `crates/voice-agent-server/src/services/knowledge/` — `processing.rs`, private policy/assessment tests, narrow Wiki integration.
- `crates/voice-agent-server/src/services/background/` — new `knowledge_ingest.rs` and mode-aware admission/eligibility/scheduler/execution, reuse `models.rs`.
- `crates/voice-agent-server/src/database/jobs/` — mode-safe durable insert/pin/idempotency, reuse Job activity and events.
- `crates/voice-agent-server/src/database/` — settings + proposal metadata, next available main SQLx migration.
- `crates/voice-agent-server/src/app/admin/knowledge.rs` — new authenticated routes delegated into deep modules.
- `apps/admin-web/src/pages/knowledge/KnowledgePage.vue` — Review + Analyze, keeping Raw/Wiki/Drafts, Reading/Compose/Graph and existing editor; `KnowledgeSettingsPage.vue` — real controls on the existing `/knowledge/settings` route; i18n and tests.
- `docs/flows/13-knowledge-v3.md`, `docs/flows/14-knowledge-graph-v3.md`, `docs/knowledge/verification-v3.md` and tests — update on implementation, not prematurely claim shipped. Preserve ADR 0124 and the Graph index v3 contract.
