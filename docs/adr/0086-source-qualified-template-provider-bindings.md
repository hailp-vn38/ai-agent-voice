# ADR 0086 — Source-qualified Template Provider bindings

## Status

Accepted.

## Context

Admin-managed database Providers and deployment-configured Providers are both useful Template choices. Their raw names can collide. Copying deployment configuration into database Provider rows would introduce a second authority for credentials and drift after restart; treating all choices as runtime residents would also force unnecessary model loading.

## Decision

A Template binding retains its Provider Origin. Existing database keys remain unchanged; deployment references use `config:<kind>:<raw_key>`. Database bindings keep a restrictive Provider foreign key, while config bindings retain a raw deployment key independently of database Provider records. Changing origins replaces both reference columns atomically with Template revision and audit.

The Admin Provider configuration catalog combines both origins using safe metadata. It is distinct from the materialized Provider Catalog and Runtime Catalog. Deployment Providers remain read-only through Admin. Missing explicit config instances remain visible as unavailable bindings and fail admission without fallback; only unbound core slots inherit defaults.

Managed runtime mode resolves immutable deployment snapshots and uses the existing Deployment ProviderVersion namespace, manager budgets and Resource Leases. Metadata indexing does not materialize every instance. Legacy runtime mode rejects new config bindings with `config_provider_requires_managed_runtime`; an existing config binding fails closed there. Supporting selective legacy loading after Admin binding would require a separate lifecycle decision.

## Consequences

The forward migration preserves database binding identities and SQLite integrity. Config rename/removal requires explicit rebinding; it never deletes Template relationships automatically. This extends ADR-0052 and ADR-0071 while retaining immutable per-session profiles and forward-only migrations under ADR-0063. Web components stay unchanged; existing config edit/delete controls receive a read-only API error.
