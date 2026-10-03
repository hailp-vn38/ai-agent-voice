# 05: Database-backed Device admission

**What to build:** Voice Protocol Client được admit hoặc reject trước WebSocket upgrade theo Device/Agent database policy, trong khi database-disabled và admission-disabled flow vẫn giữ server behavior hiện có.

**Blocked by:** 02: Admin control-plane shell và Agent/Device CRUD.

**Status:** completed

- [x] Admission activates only when database and device admission are explicitly enabled; unknown/disabled Device returns 403 before upgrade, database/pool/busy/resolver/profile failure returns coarse 503 and never falls back silently.
- [x] Admission resolve binds Device to enabled Agent and uses server defaults when the Agent has no Template assignments; no DB lookup remains in the realtime actor after acceptance.
- [x] Dev/migration auto-register requires one configured enabled Agent, creates one enabled Device with safe metadata atomically and handles same-identity races without storing credentials/headers/client hello.
- [x] Effective admission baseline remains stable after DB failure or Admin mutation; reconnect/new connection re-resolves policy.
- [x] WebSocket tests prove database-disabled compatibility, 403/503 pre-upgrade behavior, auto-register race safety, no-assignment server defaults and immutable admitted baseline.

## Comments

- 2026-09-29: Implemented explicit pre-upgrade Device admission and immutable connection-owned admission snapshot. Added database-disabled, identity bound, 403/503 database-pool/profile, no-assignment defaults, auto-register race and reconnect-policy WebSocket coverage. `cargo check -p voice-agent-server`, targeted admission/bootstrap tests and `cargo test --workspace` pass.
