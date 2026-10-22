# Knowledge V3 — Physical Folder Tree implementation

- Target branch: feat/knowledge-memory-v3-file-first-index
- Status: implemented in Rust Core / authenticated Admin adapter / Admin Web; CI verification pending.
- Decision: Explorer A with real user-selected physical directories; no virtual path catalog.
- ADR: [0122](../adr/0122-knowledge-physical-folders.md)

## Canonical storage

Raw working notes are editable Markdown under knowledge/raw/<directories>/<name>.md. They are not Source until explicitly imported. Immutable Source snapshots retain knowledge/raw/src_<id>/1/{original.md,extracted.md,manifest.json}. Frontmatter Wiki records can use user-selected names and folders under knowledge/wiki/. For a published page, pending edits are staged at the corresponding path under knowledge/drafts/. Stable Wiki ID in frontmatter survives a move; history remains keyed by stable ID under revisions/ and draft_revisions/. Existing flat Wiki and draft pages remain supported. Memory remains separately owned.

## Trusted Rust interfaces

- KnowledgeCore::tree() preserves the all-roots snapshot for existing callers. KnowledgeCore::tree_in(Some(space)) returns only the requested physical root (raw, wiki or drafts); invalid names fail closed. Raw-only browsing avoids Wiki/Drafts scans. Wiki/Drafts still cross-check staged ownership. A partial scan or symlink fails closed.
- KnowledgeCore::create_folder() accepts a relative multi-component `name` (e.g. `docs/flow`) and creates missing parent directories inside Raw or Wiki in one server request.
- KnowledgeCore::raw_create() accepts nested `file_name` (e.g. `docs/flow/ws-flow.md`) relative to `folder_path`, validates the complete target and creates missing directories inside Raw before the atomic Markdown write. The final file must not already exist. Raw read/save/move retain expected_sha256 CAS and validated root-qualified paths. No folder or file may escape its root or traverse symlinks.
- KnowledgeCore::raw_import() snapshots checked Raw bytes using existing SourceVault::import_markdown and unchanged src_id:1 format.
- KnowledgeCore::put_draft_at() extends Admin Wiki creation with optional path without changing existing WikiInput and Agent Job callers. Existing put_draft() remains backward compatible.
- MarkdownVault performs recursive physical scans for Wiki/Drafts, flat scans for Memory facts, rejects duplicate stable IDs, preserves revision CAS and trusted atomic commit, and resolves an ID from its canonical file.
- KnowledgeCore::move_page() relocates one canonical Wiki file with expected_revision and expected_folder_path. Pending staged edits must be published first.
- Agent search/read still require canonical verified Wiki and immutable Source verification; neither Raw notes nor staged drafts appear as Agent Knowledge.

## HTTP routes

| Method | Endpoint | Contract |
|---|---|---|
| GET | /api/admin/knowledge/tree | Backward-compatible unfiltered items array for raw/wiki/drafts |
| GET | /api/admin/knowledge/tree?space=raw, wiki or drafts | Only the selected root; invalid space returns 400 invalid_arguments |
| POST | /api/admin/knowledge/folders | space, parent_path, name (relative folder path, e.g. `docs/flow`; nested creation on server) |
| POST | /api/admin/knowledge/raw/files | folder_path, file_name (relative file path, e.g. `docs/flow/ws-flow.md`), body; missing directories are created on server |
| GET | /api/admin/knowledge/raw/files?path= | mutable Raw note path |
| PUT | /api/admin/knowledge/raw/files | path, body, expected_sha256 |
| POST | /api/admin/knowledge/raw/import-source | path, name, provenance, expected_sha256 |
| POST | /api/admin/knowledge/raw/move | path, target_folder_path, file_name, expected_sha256 |
| POST | /api/admin/knowledge/pages | existing fields plus optional folder_path and file_name for creation |
| POST | /api/admin/knowledge/pages/{id}/move | target_folder_path, file_name, expected_folder_path, expected_revision |

Existing Source, Wiki read/edit/publish, index health/rebuild and Memory routes are preserved. All browser-supplied paths are relative; no arbitrary filesystem path is exposed.

The Web Explorer defaults to **Wiki**, with single-choice Raw/Wiki/Drafts controls. Changing the displayed root clears the previous file/editor selection and requests only that root from the server. Raw/Drafts use local path filtering; Wiki keeps its existing full-text search. This selection is only a browser view and cannot relax Agent read policy.

## Invariants and tests

Raw imported snapshots do not track later note edits. Source bundles are read-only; Drafts cannot be mutated as standalone browser files. Stable IDs survive physical moves; index relative_path follows the on-disk file. Duplicate Wiki IDs, malformed Markdown, symlinks and partial scans fail; index recovery remains fail closed. Flat legacy Wiki and Draft paths are accepted with no migration. Indexed passages use only published canonical content.

New Rust tests exercise real nested directories, Raw CAS and import pinning, mirror staging, Agent isolation, file moves, restart/rebuild and duplicate-ID failure. Run cargo test -p voice-agent-server and npm run typecheck/build from apps/admin-web in an environment with Rust/Node dependencies before deployment. This container lacks Cargo and network access, so no successful compilation is claimed here.
