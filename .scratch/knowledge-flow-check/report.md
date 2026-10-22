# Knowledge flow verification — 2026-10-16

## Current flow

1. Authenticated Admin imports UTF-8 Markdown evidence through
   `POST /api/admin/knowledge/sources`. Source originals and checksum manifests
   are immutable.
2. Admin saves a Wiki draft through `POST /knowledge/pages` or
   `PUT /knowledge/pages/{id}`, with an expected revision.
3. Explicit `POST /knowledge/pages/{id}/publish` checks the draft revision and
   Source checksum before accepting a canonical Wiki revision and reconciling
   the SQLite FTS projection.
4. Editing a published Wiki stages a replacement separately. Agent reads keep
   using the accepted revision until the replacement is published.
5. Enabled `core.knowledge` exposes only `knowledge.search` and `knowledge.read`
   to Agent Jobs. These operations exclude drafts and verify Source evidence.
   The group is disabled by default; Admin access is independent of this policy.
6. Restart reconciliation and explicit index rebuild recover derived search
   data. Tests cover missing vault roots, tampered evidence, revision conflicts,
   staged edits across restarts, and complete Admin reads of long documents.

## Added coverage

`apps/admin-web/src/pages/knowledge/KnowledgePage.test.ts` adds two component
tests through the real HTTP client with mocked fetch responses:

- Import Source → select its reference → save draft → publish expected revision
  → search; publication action disappears after acceptance.
- Open an existing draft → edit → submit its expected revision → receive HTTP
  409; the editor retains the pending text and allows another action.

## Findings

- [P1: Publish can accept an older draft than the editor displays](issues/01-publish-unsaved-editor.md).
- [P2: Admin search labels drafts as verified](issues/02-search-draft-status.md).

These findings come from tracing the implementation and API contracts; neither
has been reproduced against a running deployment. Production behavior was not
changed in this verification task.

## Limits

Component tests mock HTTP responses; they are not browser-to-server end-to-end
tests. No live deployment, real LLM session, production data, external provider,
or hardware was exercised. Workspace-wide Rust tests were not requested.


## Web verification and fix — 2026-10-19

The current Knowledge component suite supersedes the historical coverage above:
it uses a mocked API client, not real HTTP. Added two regression cases for
pending title/body edits; both failed before the fix and passed afterward.
Publish now requires saving pending content, explains that requirement in both
languages, and displays the accepted API record after publication. Issue 01 is
resolved. The current search template no longer renders the incorrect fallback
status label described in issue 02; no additional change was needed there.

The local dependency installation lacked Cytoscape despite correct manifest and
lockfile entries. `npm ci --no-audit --no-fund` restored the pinned dependencies
without changing either file. All commands below ran via `scripts/agent-test.sh`:

- Focused Knowledge tests: 1 file / 3 tests passed.
- Full web suite (`--maxWorkers=4`): 65 files / 305 tests passed.
- Web typecheck and production build passed.
- `git diff --check` passed.

Final suite log: `/tmp/ai-agent-voice-test-logs/agent-test.run.zNACCK`.
Build log: `/tmp/ai-agent-voice-test-logs/agent-test.web.FFsdb9`.
No live browser/backend E2E, Rust tests, providers or hardware were exercised.
This host runs Node 20.19.2, below the declared Node 22.12+ requirement.


## Node upgrade verification — 2026-10-19

The user's default Node is now v24.22.0 (npm 11.19.1), installed from the
official Linux x64 archive after SHA-256 verification. User commands resolve
through `~/.local/bin`; the OS-managed `/usr/bin/node` remains unchanged.
An interactive Bash shell also resolves the new version.

Rechecked via `scripts/agent-test.sh` on Node 24:

- Focused Knowledge tests: 3 passed.
- Full web suite: 65 files / 305 tests passed.
- Production build, including typecheck: passed.

Suite log: `/tmp/ai-agent-voice-test-logs/agent-test.run.fUA6vl`.
Build log: `/tmp/ai-agent-voice-test-logs/agent-test.web.Hcxo2i`.
This supersedes the Node version limitation in the preceding verification.
Browser/backend E2E remains untested.

## Remote integration — 2026-10-19

Pulled 12 remote commits through `3a1f30dd`. The remote option A workspace is
read-only and supersedes the local editor publication fix above. Resolved the
Knowledge template conflict in favor of the current reader, retaining the
Admin API documentation and replacing obsolete editor tests with a read-only
draft regression test. No import, edit or publish form was restored.

Post-integration checks via `scripts/agent-test.sh` passed:

- Focused Knowledge component tests: 1 file / 4 tests.
- Full web suite: 66 files / 309 tests.
- Production build, including typecheck.
- Git whitespace check.

Suite log: `/tmp/ai-agent-voice-test-logs/agent-test.run.T957NL`.
Build log: `/tmp/ai-agent-voice-test-logs/agent-test.web.GSvqnV`.
Rust tests and live browser/backend E2E were not run for this integration.
