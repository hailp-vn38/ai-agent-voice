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

1. Authenticate in Admin. Navigate to Knowledge; create a draft; reload and confirm it appears in the list. Unverified drafts must be absent from `knowledge.search`/`knowledge.read` Job results.
2. Import an immutable Markdown Source; publish the draft using its `src_ID:1` reference plus expected revision; verify it becomes searchable for permitted new Jobs.
3. Change Source original or attempt to publish with an unknown/invalid version; publication must fail.
4. Create an unconfirmed Memory Fact; verify it appears in the Admin list but not Agent recall/summary. Confirm it, then ensure search and generated summary include it.
5. Forget the Fact; ensure Agent recall and summary omit it, attempting to edit the same forgotten ID is rejected, and Admin can still see its tombstoned metadata.
6. Temporarily remove the canonical Wiki/Fact root during rebuild: it must fail closed, mark its index stale, and preserve the prior indexed rows. Restore the root; rebuild and verify recovery.
7. Verify two built-in groups `core.knowledge` and `core.memory` are disabled by default. Enable via authenticated Core Plugin policy; existing persisted Job frozen choices remain valid and do not silently gain new tools.

## Unshipped acceptance

Binary PDF/DOCX/image/audio/video ingest, source extraction adapters, automatic Job proposal/approval/publish, Voice memory prompt wiring, embeddings, multi-owner policy and cross-process vault locking remain outside this branch's implemented scope.

**Verification policy:** Check the exact branch HEAD CI results before merging; successful checks on earlier commits are not proof that HEAD is green. The CI `ci.yml` Rust test and `admin-web.yml` web checks are mandatory. A temporary formatter workflow must be removed before merge.
