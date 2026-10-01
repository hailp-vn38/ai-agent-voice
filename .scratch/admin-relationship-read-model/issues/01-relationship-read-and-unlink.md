# 01: Admin relationship read và unlink

Type: task

Status: resolved

**What to build:** Authenticated Admin API can read active Agent–Template and Template–Provider relationships, read inverse usage, and unlink either relationship with optimistic concurrency and an audit record.

## Acceptance criteria

- [x] `GET /agents/{agent_key}/templates` returns active assignments with Template identity and `is_default`.
- [x] `GET /templates/{template_key}/agents` returns active users with Agent identity and `is_default`.
- [x] `GET /templates/{template_key}/providers` returns every persisted Provider binding; `GET /providers/{provider_key}/templates` returns its Template usage.
- [x] `DELETE /agents/{agent_key}/templates/{template_key}` rejects a default assignment, otherwise soft-unlinks it with Agent `If-Match`, revision increment and audit.
- [x] `DELETE /templates/{template_key}/providers/{provider_type}` removes the binding with Template `If-Match`, revision increment and audit.
- [x] Router-seam tests prove positive results plus `If-Match`/default conflict behavior; no endpoint changes runtime state.

## Comments

- 2026-10-01: Scope extracted from the reviewed Admin API gap document. Resource deletion, device overrides and Vision remain explicitly uncontracted.

## Answer

- Implemented six authenticated relationship routes, with owner revision supplied by every GET response. Inverse usage totals are counted before pagination.
- Agent–Template unlink is a soft unlink and rejects the active default; Template–Provider unlink is transactional. Both enforce `If-Match`, increment the owner revision and write bounded audit metadata.
- Relationship handlers live in `app/admin/templates/relationships.rs`; no handler changes `RuntimeCatalog`, resolves a secret or mutates an admitted session profile.
- Validation passed: `cargo fmt --check`, `git diff --check`, `jq empty docs/api/00-all-apis.postman_collection.json`, `cargo test -p voice-agent-server --test admin_api`, and `cargo test --workspace --quiet -- --test-threads=1`.
