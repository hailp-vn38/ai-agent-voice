# ADR 0122 — Physical Knowledge folders with immutable Source snapshots

- Status: Accepted
- Date: 2026-10-18
- Scope: Knowledge V3 physical folder explorer, Rust Core and Admin Web
- Related: ADR 0118, 0119, 0120 and 0121

## Context

Knowledge V3 stored Wiki and staged Markdown in flat stable-ID filenames. Admin could search/edit but not preserve user-selected physical folders. Editable Raw working notes cannot themselves be treated as Source: doing so would allow subsequent edits to invalidate citations. A separate path database would compete with the file-first vault and rebuildable FTS.

## Decision

1. Raw, Wiki and Drafts use real nested physical folders. The Admin tree is built from a bounded filesystem scan, never from a virtual SQLite folder hierarchy.
2. Ordinary Raw Markdown is editable under SHA-256 CAS. Existing immutable Source bundles under raw/src_<id>/1/ retain original.md, extracted.md, manifest.json and source-ref src_id:1. Importing a Raw working note creates an immutable snapshot; later note edits do not change the cited Source.
3. Wiki identity stays in frontmatter and is independent of filename/path. Existing flat Wiki and Draft files stay valid without migration.
4. Pending edits of published Wiki live at corresponding nested physical paths under Drafts, independently of canonical Wiki; Agent/FTS continues to read source-verified canonical only. Existing revision CAS, immutable history, atomic commit and Source validation remain in Rust Core.
5. MarkdownVault resolves stable IDs through a complete physical scan rather than a second durable path catalog. Fail closed on duplicate ID, symlink, malformed Markdown, unavailable root or partial scan. Do not mass-prune on failure.
6. Admin mutations accept root-qualified relative paths only, with single trusted filesystem seam. No arbitrary writes into Source bundles, Drafts, history, or Memory.
7. Individual file move uses CAS and no-clobber checks. A Wiki with an active staged edit must publish before move. Folder move/delete and drag-drop are deferred.
8. Memory and three SQLite database lifecycles stay separate; no new Agent Job operations or database are introduced.

## Alternatives

Virtual folders were rejected because the UI would disagree with actual files. Editing immutable Source bundles was rejected to protect provenance. A fourth authoritative path catalog would increase synchronization and recovery complexity. Raw/Wiki/Draft storage is not unified because the domains have different trust policies.

## Verification

Exercise nested path creation, Raw edits/snapshot pinning, mirrored draft isolation, source-verified publish, stable ID after move/restart, legacy flat files, conflicts, symlink rejection and incomplete-scan fail-closed behavior. Run Rust tests and Admin typecheck/build before release.
