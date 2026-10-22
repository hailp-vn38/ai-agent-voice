# Publish can accept an older draft than the editor displays

Status: resolved
Priority: P1

## Reproduction

1. Save a Wiki draft with body A and select a valid imported Source.
2. Edit its body to B without saving again.
3. Click Publish.

The Publish button remains enabled. `KnowledgePage.vue` sends only
`expected_revision` and `source_ref`; `KnowledgeCore::publish` accepts the saved
draft A. The editor continues showing B with a publication success notice until
the page is reopened. This can mislead the administrator about the evidence
actually published.

## Evidence

Static flow inspection: `apps/admin-web/src/pages/knowledge/KnowledgePage.vue`
(`publish`, editor buttons), and
`crates/voice-agent-server/src/services/knowledge/wiki.rs` (`publish`).
This finding has not been reproduced against a running server/browser.

## Expected behavior

Require saving pending edits before publication, or explicitly publish those
edits with the existing revision checks. The editor must show the accepted
content after success.

## Resolution — 2026-10-19

Reproduced with component tests for pending title and body edits. Publish now
checks unsaved editor content in both its handler and disabled state, with an
English/Vietnamese save-first hint. After publication, the editor uses the
accepted API record. Reverting edits or saving them enables publication again.
Tests exercise save with revision 1, then publish with revision 2.

## Remote integration — 2026-10-19

Remote commit `3a1f30dd` includes the read-only option A workspace, which removes
the editor and publication controls. The earlier local editor fix and its tests
are superseded by that interface. The merged regression test checks that drafts
render their saved content without editable fields, forms or write requests.
The Admin API still requires saving the intended draft before publication.
