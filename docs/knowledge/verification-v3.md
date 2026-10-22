# Knowledge + Memory V3 — verification and merge gates

Current scope: file-first Markdown Wiki/Memory, two independently migrated/rebuildable FTS5 databases, authenticated Admin, source-verified Markdown publication, read-only frozen Core Job V3 adapter and two Vue workspaces.

## Rust checks

```bash
scripts/agent-test.sh run -- cargo fmt --check
scripts/agent-test.sh run -- python3 -m unittest discover -s scripts/tests -p test_sqlx_placement.py
scripts/agent-test.sh run -- python3 scripts/check-sqlx-placement.py
scripts/agent-test.sh run -- cargo clippy --all-targets --all-features -- -D warnings
scripts/agent-test.sh targeted services::knowledge
scripts/agent-test.sh targeted tools::core
scripts/agent-test.sh targeted services::background::tests::core_policy
scripts/agent-test.sh full
```

The production SQLx queries must remain in `src/database/knowledge_index.rs`, not under `services/`. The application SQLx migrations apply only to the main DB; Knowledge and Memory indices have separate `sqlx::migrate!` roots.

## Admin Web checks

```bash
npm --prefix apps/admin-web ci
scripts/agent-test.sh web typecheck
scripts/agent-test.sh web test
scripts/agent-test.sh web build
```

## Manual acceptance

1. Authenticate in Admin. Navigate to Knowledge; create a draft; reload and confirm it appears in the list. Unverified drafts must be absent from `knowledge.search`/`knowledge.read` Job results. Create a Wiki body with over 10,000 short lines, open it through the editor and save without truncating data.
2. Import an immutable Markdown Source; publish the draft using its `src_ID:1` reference plus expected revision; verify it becomes searchable for permitted new Jobs.
3. Change Source original or attempt to publish with an unknown/invalid version; publication and subsequent Agent read/search must fail closed and mark Knowledge stale. Restore the Source and rebuild to recover.
4. Create an unconfirmed Memory Fact; verify it appears in the Admin list but not Agent recall/summary. Confirm it, then ensure search and generated summary include it.
5. Forget the Fact; ensure Agent recall and summary omit it, attempting to edit the same forgotten ID is rejected, and Admin can still see its tombstoned metadata.
6. Stop the server, temporarily remove the Wiki or Fact root, then restart: initialization must fail closed instead of recreating an empty root or pruning the index. Restore the root; restart and verify recovery. Simulate a partially written revision staging file and confirm the immutable retained revision remains complete.
7. Verify two built-in groups `core.knowledge` and `core.memory` are disabled by default. Enable via authenticated Core Plugin policy; existing persisted Job frozen choices remain valid and do not silently gain new tools.

8. Publish one Wiki, then edit it. Admin must show a staged draft; Agent `knowledge.read` and `knowledge.search` must continue returning the original verified text. Restart and repeat. Publish the staged revision using expected staging CAS; Agent must now see only the new version, with original evidence archived.
9. Confirm `knowledge.search` and `knowledge.read` tool results include `source_ref`, and bounded reads include `from_line`. Check a stale/tampered Source does not return an apparently valid citation.
10. Confirm both Admin and Background Job use the same Knowledge Runtime cell/pools in a running server; concurrent searches and updates must not create competing runtimes.

11. After a successful server initialization, stop the server and temporarily rename the **entire** `knowledge/` folder while leaving the main SQLite database intact. Restart must reject the missing vault because the top-level `.knowledge_v3_initialized` guard remains. Restore the folder and confirm the existing Wiki is still present.

## Unshipped acceptance

Binary PDF/DOCX/image/audio/video ingest, source extraction adapters, Voice memory prompt wiring, embeddings, multi-owner policy and cross-process vault locking remain outside this branch's implemented scope. Knowledge AI processing now has backend proposal/review/auto-publish APIs and persisted Settings; see [AI ingest verification](ai-ingest-verification-v3.md). Review/Analyze UI outside Settings is excluded by the implementation request. Remote model/deployment/process-crash acceptance remains separate.

**Verification policy:** Check the exact branch HEAD CI results before merging; successful checks on earlier commits are not proof that HEAD is green. The CI `ci.yml` Rust test and `admin-web.yml` web checks are mandatory. A temporary formatter workflow must be removed before merge.

## QMD-inspired Markdown query/passage regression

Run the repository's test wrapper (rather than bare `cargo test`) for the pure modules and Core regression tests:

```bash
scripts/agent-test.sh targeted markdown_chunker
scripts/agent-test.sh targeted knowledge_lex
scripts/agent-test.sh targeted qmd_markdown_passages
scripts/agent-test.sh targeted qmd_lexical_fallback
scripts/agent-test.sh full
```

Cover safe FTS5 syntax with Unicode Vietnamese, quoted phrases, `DEC-0054` / `gpt-4`, token-prefix semantics, and invalid input. Verify relevant passage is selected when the match is far beyond the first 600 characters, body-relative `start_line/end_line` span the evidence, valid `source_ref` is attached, and an index rebuild/restart reproduces the result. Check that a draft Wiki, a staged edit, or an unconfirmed/forgotten Memory fact is never exposed through Agent search (including fallback OR). Confirm the migration uses only `knowledge_index.db` and that a complete forced rebuild recreates chunk FTS.

The title-only snippet regression `search_excerpt_uses_matching_title_when_body_does_not_contain_query` asserts that both verified Knowledge and confirmed Memory return evidence in the excerpt even if the body does not contain the match. The forced passage-repair regression `qmd_forced_rebuild_restores_deleted_chunk_fts_rows_without_rewriting_wiki` removes only derived FTS rows, rebuilds, then checks that original Markdown bytes remain identical.

## Search-citation review regression

A title-only Knowledge search must return `match_field=title` and no body-line citation; a body-only match must retain correct 1-based `start_line/end_line`. Cross-field `mixed` matches must omit body-only line claims. These checks complement the passage ranking and existing immutable Source verification gates.


## 2026-10-19 post-spec branch reconciliation gates

**Status:** The Graph/Explorer implementation is committed; native Obsidian interactive validation remains a separate gate. `/knowledge/settings` now persists AI settings, with backend Analyze/Review/Auto Publish implemented; [AI ingest verification](ai-ingest-verification-v3.md) separates local checks from deployment acceptance. The [branch audit](ai-ingest-branch-reconciliation-2026-10-19.md) compares 145 commits after the AI Ingest spec baseline.

### Committed Graph and Admin Web regressions

Verify the code paths which are now integration dependencies of any future AI Ingest implementation:

```bash
scripts/agent-test.sh targeted wiki_links
scripts/agent-test.sh targeted wiki_graph
scripts/agent-test.sh targeted local_graph
scripts/agent-test.sh targeted services::knowledge
scripts/agent-test.sh web test
scripts/agent-test.sh web typecheck
scripts/agent-test.sh web build
```

The Rust checks must validate the `0003_wikilink_graph.sql` derived `kb_links` projection, v3 index migration/rebuild, missing/ambiguous/invalid links, draft visibility separation, local/global traversal and 409 link-aware move safety. The Vue checks must retain Raw/Wiki/Drafts physical selector, Reading/Compose/Graph modes, note creation with draft semantics, dirty editor guard, safe Wikilink rendering and existing gear → `/knowledge/settings` route. `KnowledgeSettingsPage.test.ts` currently asserts a **preview only**, with no editable settings.

Then perform [the Obsidian smoke procedure](obsidian-graph-smoke-v3.md) with a **copied** `knowledge/wiki/` Vault. Passing unit tests alone does **not** certify Obsidian interoperability. No tests are claimed to have been executed by this documentation update.

### Additional gates before enabling proposed AI Auto Publish

1. `knowledge_ingest_v3` admission uses exactly one Knowledge-selected, qualified LLM provider pin and never rewrites global Agent Job Orchestrator/Worker settings.
2. The strict schema rejects malformed or unqualified provider outputs; a source over the bounded one-turn input budget is rejected, not silently truncated.
3. Every candidate claim has immutable Source quote/span/hash checks; semantics not confidently entailed go to Review.
4. Generated `[[Vault/Relative/Page]]` links are checked using the existing canonical Wiki resolver. Missing/ambiguous/draft-only/unverified target, unsafe path, unresolved interpretation or stale index ⇒ Review, **not** Auto Publish.
5. A Graph node existing in the Admin index is **not** proof of verified publication; Agent read/search still exclude draft canonical pages and staged replacements.
6. User-edited Compose drafts must not be overwritten; revalidation and expected revision/CAS are required for both auto and human Approve.
7. Crash/restart/retry fault injection confirms exactly one canonical publish, preservation of Source provenance, recoverable `kb_links`/FTS projection and no duplicate proposal after lost DB acknowledgement.
8. Knowledge Settings actual GET/PUT uses the **existing `/knowledge/settings` route** plus authenticated, versioned backend settings. Old example-only tests are updated to verify real controls without breaking navigation.
9. Existing `hierarchical_v3`, `plugin_operation_v3`, Core Knowledge read-only tools, Graph UI, Memory and application database lifecycles remain unchanged.

**Release rule:** all relevant tests must run against the implementation branch's **new HEAD** after coding. This document records required checks, not test results.

## Knowledge AI processing verification

[AI ingest verification](ai-ingest-verification-v3.md) records the settings/analyze/review contracts, single-provider Job mode, publication intents, strict adapter fixtures and source/index failure cases. Existing explorer/editor/Graph layouts are preserved. Defaults remain disabled, review-all and manual; the implementation does not enable processing or auto publish in a deployment.
