# Admin search labels drafts as verified

Status: needs-triage
Priority: P2

## Reproduction

1. Create and save an unpublished Wiki draft containing a unique phrase.
2. Search for that phrase in the Knowledge workspace.

Admin search includes canonical drafts, but `SearchHit` has no `status` field.
The frontend renders `item.status ?? "verified"`, so the draft result is labeled
verified. The unfiltered Admin list includes the correct status.

## Evidence

Static contract inspection:
`crates/voice-agent-server/src/database/knowledge_index.rs`
(`SearchHit`, `search_with` with `include_drafts`) and
`apps/admin-web/src/pages/knowledge/KnowledgePage.vue` (result status fallback).
This finding has not been reproduced against a running server/browser.

## Expected behavior

Return the actual publication state for Admin search results, or omit the state
label when the API does not provide one. Preserve the Agent verified-only search
policy.
