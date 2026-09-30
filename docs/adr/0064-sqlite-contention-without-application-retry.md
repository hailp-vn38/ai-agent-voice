# ADR 0064 — SQLite Contention không có Application Retry

## Status

Accepted

V1 chỉ dùng `PRAGMA busy_timeout` (`1..=30_000` ms, default 5.000) để chờ SQLite lock contention. `SQLITE_BUSY`/`SQLITE_LOCKED` sau timeout là `database_busy`; SQLx pool acquire timeout là `database_pool_timeout`; I/O/storage failure là `database_unavailable`. Không application retry/backoff, retry transaction hoặc retry admission query.

Admin busy trả `503 database_busy`; pool/storage trả `503 database_unavailable` nhưng telemetry giữ taxonomy nội bộ. DB-backed admission fail `503` trước upgrade và không fallback. HistoryWriter drop best-effort, maintenance abort current iteration rồi đợi scheduled run sau. SessionActor không chờ SQLite; transaction chỉ giữ DB atomic work, không external await.
