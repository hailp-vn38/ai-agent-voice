# SQLx domain ownership

Production SQLx query construction and execution belong under
`crates/voice-agent-server/src/database/`. The existing `Database` shares one SQLite
pool across these modules; direct, typed methods keep each transaction with its
owning domain. There is no generic repository layer or additional pool.

| Owner | Persistence responsibility |
| --- | --- |
| `agents.rs` | Agent rows, conditional updates and success/conflict audit |
| `providers.rs` | Provider rows, list filters/facets, relationship counts and desired-state snapshots for startup, prewarm, diagnostics and status |
| `templates/mod.rs`, `templates/relationships.rs` | Template rows, provider bindings and Agent assignments/default selection/unlink |
| `devices.rs`, `device_enrollments.rs` | Device rows, valid Template overrides, enrollment cancellation and atomic Admin Enrollment Claim |
| `speakers/mod.rs`, `speakers/policy.rs` | Speaker profiles, voiceprint metadata, catalog publication, Agent identification policy and candidates |
| `speakers/captures.rs` | Accepted capture reservation, atomic promotion/replacement and consumed capture handling |
| `speakers/observations.rs` | Admission-time policy/candidate facts and embedding validation |
| `history/queries.rs` | Bounded Persistent Transcript reads and explicitly scoped, audited History Purge |
| `mcp_servers.rs`, `tool_allowlist.rs` | External MCP configuration/bindings, observed contract review and dispatch-time permission checks |
| `deletion.rs`, `audit.rs` | Guarded dependency-aware deletion and audit writes shared within database modules |

The database root retains connection options, migrations and readiness. Existing
admission, Provider Load Plan, archive writer/retention and credential modules retain
their responsibilities. `DesiredProvider` remains available through the database
root; existing session speaker resolver exports remain available, now accepting a
`Database` instead of exposing its pool.

Admin handlers parse/authenticate/validate requests and map typed database outcomes
to the existing HTTP bodies, statuses and ETags. They also own service-level
post-commit actions, including prewarm and Speaker snapshot invalidation. The
`writes::WriteError` variants preserve the distinctions existing endpoints make;
in particular, endpoint-specific SQL failure/conflict behavior has not been
normalized into a new policy.

Publication protection for MCP configuration, binding/review changes and shared
deletions stays held through transaction commit and required session invalidation.
External tool dispatch rechecks the current reviewed contract in SQLite, rejects
query failures, and checks its cancellation token before and after that read.
Speaker capture inference and scoring remain in workers/session, outside database
transactions. History handoff remains non-blocking and never supplies Dialogue
History. Resource Credential encryption, write-only inputs and environment fallback
remain unchanged under ADR 0083.

`Database::pool()` remains available to integration fixtures and existing low-level
database modules. Production callers outside database do not construct queries or
own SQLite transactions. `scripts/check-sqlx-placement.py` enforces this in CI,
including imported/renamed query constructors and `query*_with`. It removes only
explicit test-module bodies and test-only external modules; a file also imported by
production remains checked. The check is a conservative source-text gate, not a
Rust compiler or an AST framework; unexpected source syntax must be inspected and
the gate extended rather than exempting production queries.

This is a reversible organization change. It introduces no schema, route, wire
contract, domain terminology or authorization-policy change, so the single-context
vocabulary in `CONTEXT.md` and accepted ADRs remain the source of truth.

The baseline inventory and verification report are in
[the local issue-tracker record](../.scratch/sqlx-domain-modules/inventory.md) and
[the implementation report](../.scratch/sqlx-domain-modules/implementation.md).
