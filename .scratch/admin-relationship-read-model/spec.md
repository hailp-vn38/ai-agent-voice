# Admin relationship read model

## Goal

Expose the persisted Agent–Template and Template–Provider graph through the authenticated Admin API so the Web Admin never has to reconstruct or retain relational state locally.

## Scope

- Read active Template assignments for an Agent and active Agent users of a Template.
- Read Template Provider bindings and Templates using a Provider.
- Soft-unlink an active Agent–Template assignment and unlink one Template–Provider binding, both with owner `If-Match`, owner revision increment and bounded audit metadata.
- Keep all data in Database Desired Configuration; do not mutate `RuntimeCatalog`, resolve a secret, or hot-reload an admitted Voice Session.

## Explicit non-goals

- Resource DELETE/archive policy, Device Template Override, Vision Provider integration, system status and list filtering/facets.
- Any live RuntimeCatalog refresh or session-profile mutation.

## Public seam

The authenticated `/api/admin` HTTP router, verified by `crates/voice-agent-server/tests/admin_api.rs`.
