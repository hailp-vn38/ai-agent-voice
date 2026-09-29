# 07: Provider Load Plan và Effective Session Profile

**What to build:** New Device-admitted connection resolves a complete immutable Effective Session Profile against RuntimeCatalog, with required/optional Provider load semantics and fail-closed default Template behavior.

**Blocked by:** 05: Database-backed Device admission; 06: DB Provider → RuntimeCatalog materialization.

**Status:** resolved

- [x] Startup derives Provider Load Plan from the persisted Device → Agent → Template → Provider graph before runtime construction, then applies it to the DB Provider → RuntimeCatalog bridge: server defaults and enabled Agent default Template providers are required; enabled non-default-only providers are optional; unbound providers do not resolve secrets/build runtime. The plan must not infer DB requirements from `AppConfig::effective_agent`.
- [x] Required provider config/secret/load failure blocks startup before bind; optional failure is unavailable and excludes only its non-default candidate; startup validates enabled unbound provider config without blocking boot.
- [x] Resolver materializes Device, Agent, default Template or Server Default Profile, loaded provider bindings, prompt/language and revision into Effective Session Profile without live repository reference in SessionActor; it replaces Ticket 05's temporary assignment-count `503` gate rather than extending it.
- [x] Agent with any assignment row (including a disabled assignment) has entered the Template mechanism and never falls back to server defaults. It must have exactly one enabled default assignment; missing/disabled/invalid default Template, provider binding or loaded runtime returns coarse 503. Only Agent with no assignment row uses server defaults; invalid non-default candidates are excluded with bounded diagnostics.
- [x] Startup and WebSocket tests prove required/optional/unbound load behavior, desired-versus-loaded reporting, default fail-closed, server-default fallback and session immutability after Admin mutation.

## Comments

- Implemented `ProviderLoadPlan` (required/optional/unbound) in `database::load_plan`, applied it to the existing database provider bridge so required config/secret/load failure fails startup before bind, and left optional/unbound outcomes coarse and non-blocking.
- Replaced Ticket 05's assignment-count `503` with `EffectiveSessionProfile` in `session::profile`: any assignment row is fail-closed, only a zero-row Agent uses server defaults, and valid enabled non-default candidates are snapshotted into an immutable Template Switch Catalog with bounded reason-only diagnostics.
- VAD segmentation timing now travels with each loaded runtime, so a stored Template can bind a database VAD provider without a second lookup in the deployment TOML.
- `cargo check -p voice-agent-server --all-targets`, `cargo clippy`, targeted load-plan/admission/profile tests and `cargo test --workspace` pass. Three `speechoutput_tracer` timing tests fail identically on the pre-change tree and are unrelated to this ticket.
- 2026-09-29 review follow-up: a stored default Template binding is now proven to select a *different* already-loaded runtime, the Template Switch Catalog is proven to exclude every invalid candidate, a required provider key collision is refused before any credential is resolved, VAD timing is bundled with its runtime, and the system-prompt bound is enforced at the seam that installs it.
