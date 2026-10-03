# ADR 0051 — Admin API dùng credential riêng và optional surface

## Status

Accepted

Admin API chỉ mount tại `/api/admin` khi `api.enabled = true` và bắt buộc đúng một `Authorization: Bearer <admin_token>` với non-empty token, constant-time comparison; missing/malformed/wrong đều `401`, còn API disabled trả `404`. Token trim-rỗng làm startup fail-fast. V1 không có CORS/cookie auth/wildcard origin hay in-process brute-force limiter; proxy/network ACL chịu throttling. `admin_token` tách hoàn toàn khỏi `auth.token` của Voice/OTA và không được log, debug hoặc persist vào SQLite. `admin_audit_events` không có Admin API read surface V1.
