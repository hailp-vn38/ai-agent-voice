# ADR 0049 — Database-backed Device Admission là explicit

## Status

Accepted; các clause opt-in/conditional database đã được supersede bởi [ADR-0073](0073-required-database-and-device-admission.md). Nội dung dưới đây lưu quyết định trước đó.

Database-backed Device Admission chỉ có hiệu lực khi `database.enabled = true` và `database.devices.admission_enabled = true`; unknown hoặc disabled Device bị reject `403`, còn invalid Agent profile hoặc DB/pool/resolver unavailable bị reject `503` trước WebSocket upgrade, không fallback. Session đã admit giữ immutable profile khi DB lỗi sau đó. `database.devices.auto_register` mặc định false và chỉ dành cho dev/migration; khi bật, `auto_register_agent_key` phải resolve tới đúng một Agent enabled, Device mới `enabled=true` bind ngay Agent đó và chỉ ghi audit metadata an toàn. UNIQUE(`device_id`) cùng transaction/upsert-safe logic xử lý race connection mà không tạo duplicate hay persist credential/header nhạy cảm.

