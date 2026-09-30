# ADR 0057 — Database enabled là startup dependency fail-fast

## Status

Accepted

Khi `database.enabled=true`, config validation, SQLite open, PRAGMA, SQLx migration compatibility và forward migration phải thành công trước bind listener; không degraded startup không DB. SQLx history authoritative, migration forward-only; binary cũ gặp DB migration version mới hơn fail `database_schema_incompatible`, không auto downgrade/best effort. Backup/restore và rollback release là operator-owned. Lỗi runtime sau boot chỉ làm new DB-dependent request/session trả `503` hoặc HistoryWriter drop best-effort; Effective Session Profile đã admit và `/health` liveness không bị thay đổi. DB readiness thuộc endpoint/diagnostics riêng sau này.
