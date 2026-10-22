# Knowledge AI ingest — implementation and verification

Scope: backend P0–P3 from [implementation v3](ai-ingest-auto-publish-implementation-v3.md). Per the 2026-10-19 implementation request, UI changes are limited to the existing `/knowledge/settings` page. The Knowledge explorer, reader, composer, Graph and every other page retain their layout. Analyze and Review are authenticated APIs; a Review workspace and Source Analyze button are not part of this UI scope. P4 remains deferred.

## Implemented boundaries

- Application migrations 0123–0125 persist settings, add `knowledge_ingest_v3` to the Job contract and events, and retain proposal decision metadata. Existing hierarchical/plugin snapshots omit the optional Knowledge field. No new database, scheduler or ingest-run ledger.
- Defaults are disabled, `review_all`, manual, at most five candidates. Settings use `If-Match` CAS; selection references existing catalog providers. Global Job provider settings are independent. A disabled configuration can be saved when its previously selected provider is unavailable.
- Initially qualified adapter path: **OpenAI Chat Completions only**, with declared structured support plus runtime admission. Responses and other adapters remain unavailable for this pipeline until separately qualified. Local HTTP fixtures exercise strict schema, refusal, truncation, unexpected tools, unsupported mode and timeout. No assertion of factual truth or real remote model qualification is made by a declared flag.
- Analyze pins immutable Source version/hash, one exact provider pin, policy revision and limits. Source content must fit 40 KiB; larger Sources return `source_too_large_for_analysis` without truncation. Up to sixteen verified canonical Wiki identities/revisions are offered as update targets. Duplicate checks still consult the full vault/index; an unavailable candidate window or stale index cannot authorize auto publication.
- Structured output is independently JSON Schema validated and parsed with unknown-field rejection. One bounded repair is allowed. Model-authored tools, authority fields, foreign Source refs, unoffered update targets and unsafe folders cannot mutate Wiki.
- Direct-excerpt creates alone may auto publish. Paraphrase, uncertain evidence, duplicates, links to draft/missing pages and every existing-page update require review. Generated links reuse canonical path resolution; Graph never grants publication authority.
- Proposals store refs, offsets, lengths and SHA-256 metadata. Markdown remains in Wiki/Drafts. Evidence text is reconstructed from the immutable Source and exact draft. Human drafts are never overwritten. A staging conflict becomes `needs_reconcile`; Admin may reject its metadata to release the target, leaving all Markdown untouched.
- Approval compares proposal/draft/base revisions, Source checksum, content hash and evidence. Repeating an accepted decision returns the accepted revision without publishing a later human edit. Publishing intent precedes canonical commit. Restart compares canonical revision/hash/provenance, settles a matching commit once, and routes uncertainty to `needs_reconcile` without reapplying.
- Index failure preserves published state and accepted Markdown while search and Admin Graph refuse stale results. Rebuilding the index does not create another Wiki revision.
- `auto_after_import` runs only after immutable Source import (including explicit Raw import), never Raw save. The import ACK retains `201` if enqueue fails and exposes `analysis_error`; explicit Analyze can retry. Import and Job admission are separate commits: a process exit in the interval before enqueue is not an atomic import-and-analysis guarantee.

## Admin API

Existing Admin Bearer authentication applies to every route.

| Route | Contract |
| --- | --- |
| `GET /api/admin/knowledge/settings` | Settings, revision, provider options and readiness. |
| `PUT /api/admin/knowledge/settings` | Settings body and `If-Match: "<revision>"`. |
| `POST /api/admin/knowledge/sources/{id}/analyze` | Optional `Idempotency-Key`; `202 {job_id,source_ref,replayed}`. |
| `GET /api/admin/knowledge/reviews?state=pending_review&limit=20&after=...` | Stable ID cursor, bounded metadata. |
| `GET /api/admin/knowledge/reviews/{id}` | Exact staged/canonical records, Source and reconstructed evidence spans. |
| `POST /api/admin/knowledge/reviews/{id}/approve` | `{expected_revision,draft_revision,canonical_base_revision}`. |
| `POST /api/admin/knowledge/reviews/{id}/reject` | Same CAS body; `draft_revision:null` is valid for an unstaged reconciliation conflict. |

Stale decisions return 409; invalid Source evidence returns 422; unavailable providers return 503. Review results link through `job_id` to the existing Job timeline. Explicit Job-history deletion retains proposal audit metadata, so that Job link may later be unavailable.

## Deterministic checks

Use the repository wrapper for every check:

```sh
scripts/agent-test.sh run -- cargo test --quiet -p voice-agent-server --lib knowledge_
scripts/agent-test.sh targeted services::knowledge
scripts/agent-test.sh targeted services::background
scripts/agent-test.sh targeted tools::core
scripts/agent-test.sh check
scripts/agent-test.sh run -- python3 scripts/check-sqlx-placement.py
scripts/agent-test.sh run -- cargo fmt --all --check
scripts/agent-test.sh run -- cargo clippy -p voice-agent-server --all-targets -- -D warnings
scripts/agent-test.sh web typecheck
scripts/agent-test.sh web test
scripts/agent-test.sh web build
scripts/agent-test.sh full
```

Tests cover separate provider admission with global Background disabled, CAS/defaults, authenticated import/analyze/review/approve, post-import trigger, schema/authority/path rejection, bounded repair, duplicate admission, cancellation on disable, excerpt vs inference/forged quotes/unresolved links, staged updates and human collisions, idempotent approval, restart checkpoints and degraded-index behavior. The tests use real temporary SQLite/vault files, fake Job models and local real-adapter HTTP fixtures. Restart fault tests recreate the Background/Knowledge runtimes from persisted state; they are not production-process kill tests.

Remote LLM smoke, deployment startup/restart under the deployment's actual credentials and production-process crash qualification remain operational acceptance boundaries. Defaults stay disabled/review/manual; this document does not assert deployed acceptance or enable auto publish in any deployment.

## Local execution results — 2026-10-19

Full Rust workspace: **971 passed, 0 failed, 13 existing ignored** across 61 test groups. Full Web suite: **71 files, 328 tests passed**. Workspace Clippy with all targets/features and warnings denied, Rust formatting, SQLx placement, Web typecheck and production build all passed. Targeted Background regression passed 150 tests with one existing ignored deployment test.

Independent code review: Standards found no mandatory violation and one nonblocking suggestion to type internal state strings; Spec found no actionable discrepancy within the amended UI scope. See the local [two-axis review](../../.scratch/knowledge-ai-ingest/review.md) and [execution evidence](../../.scratch/knowledge-ai-ingest/verification.md).

The ignored tests require external model/runtime/deployment prerequisites; they are not counted as passed. Remote LLM smoke and production deployment/crash qualification remain pending. Defaults remain disabled/review/manual.
