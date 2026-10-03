# 01: SQLite foundation và boot lifecycle

**What to build:** Operator có thể bật SQLite control plane trên một local database path duy nhất và server chỉ bind khi configuration, schema compatibility, PRAGMA và forward migration hợp lệ; khi database tắt, Voice behavior hiện tại không đổi.

**Blocked by:** None (can start immediately).

**Status:** ready-for-human

- [x] Database config opt-in, single-owner local-filesystem contract, validated pool/busy-timeout/migration/shutdown settings và no implicit Agent/Template/Device seed được áp dụng; NFS/SMB, active-active và multi-process writer không được hỗ trợ.
- [x] SQLite mở với WAL, foreign keys, synchronous normal và busy timeout; SQLx migration history là authoritative, forward-only, schema newer-than-binary/pending migration khi migration-on-start tắt/migration failure đều fail trước listener.
- [x] `/health` là process liveness; `/ready` có seam application-owned nhưng DB-1 không đổi WebSocket admission, provider resolution, Template, MCP hoặc history runtime.
- [x] Startup/shutdown có graceful deadline validated; shutdown chặn listener mới trước và không tạo retry/backoff application-level cho SQLite.
- [x] Public startup tests chứng minh fresh DB migrate/boot, migration idempotency, incompatible schema fail trước bind, migration failure fail trước bind và database-disabled legacy boot.

## Comments

- Implemented in `fd39612` (`feat: add SQLite startup foundation`). Targeted database/config/WebSocket lifecycle tests, `cargo check -p voice-agent-server`, `cargo fmt --check`, and `git diff --check` pass. `cargo test --workspace` remains blocked by three existing failures in `speechoutput_tracer` unrelated to this ticket.
