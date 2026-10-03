# 01: Device override và Admin P1 read model

Type: task

Status: resolved

**What to build:** Persist and resolve a Device Template Override, then expose bounded system and
catalog read models through the authenticated Admin API.

## Acceptance criteria

- [x] Device create/get/list/patch exposes nullable `template_key`; an override must be an enabled
  Template assigned to the Device Agent, and clearing it restores Agent-default selection.
- [x] Admission uses the override as the required profile while keeping admitted profiles immutable.
- [x] `GET /api/admin/system` reports safe operational state only.
- [x] Provider/Template list filters, totals and facets are bounded and truthful.
- [x] Router-seam tests prove each contract; no Admin route resolves secrets or mutates loaded runtime.

## Comments

- 2026-10-01: Vision Provider remains deferred because its domain contract is not approved in the
  source document.
- 2026-10-01: Validated by `cargo fmt --check`, `git diff --check`, focused Admin/profile tests,
  and `cargo test --workspace --quiet -- --test-threads=1`.
