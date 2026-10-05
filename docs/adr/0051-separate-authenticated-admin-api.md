# ADR 0051 — Admin API dùng credential riêng và optional surface

## Status

Accepted

Admin API chỉ mount tại `/api/admin` khi `api.enabled = true` và bắt buộc đúng một `Authorization: Bearer <admin_token>` với non-empty token, constant-time comparison; missing/malformed/wrong đều `401`, còn API disabled trả `404`. Token trim-rỗng làm startup fail-fast. CORS cho browser trong trusted LAN chấp nhận HTTP/HTTPS origin tại localhost, loopback, private IP và link-local IP (IPv4/IPv6), phản hồi origin cụ thể và xử lý OPTIONS preflight trước authentication. Actual request vẫn bắt buộc admin token; không bật cookie credentials hay wildcard origin. Các method GET/POST/PUT/PATCH/DELETE/OPTIONS và request headers của preflight được hỗ trợ; ETag và X-Request-Id được expose. V1 không có in-process brute-force limiter; proxy/network ACL chịu throttling. `admin_token` tách hoàn toàn khỏi `auth.token` của Voice/OTA và không được log, debug hoặc persist vào SQLite. `admin_audit_events` không có Admin API read surface V1.
