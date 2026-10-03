# Admin P1 control-plane

## Goal

Complete the Admin P1 routes with deterministic desired-state semantics, without hot-reloading a
Voice Session or materializing a provider during an Admin request.

## Scoped routes

- Device Template Override: a Device may select an enabled Template already assigned to its
  enabled Agent; JSON `null` clears that override and reverts to the Agent default.
- `GET /api/admin/system`: version, uptime, database reachability, provider runtime summary and
  active session count, with no filesystem paths or secrets.
- Provider and Template lists: bounded `q`, type/language and enabled filters; accurate
  pagination `total` and Provider-type facets.

## Deferred architecture

Vision Provider is not included in this implementation. It requires a separately approved
ProviderType, adapter catalog, database validation, runtime materialization and binding contract.

## Public seam

Authenticated `/api/admin` HTTP routes, covered through `crates/voice-agent-server/tests/admin_api.rs`.
