# ADR 0072 — SQLite Device Enrollment và OTA pending contract

## Status

Accepted; nhánh legacy OTA khi enrollment tắt được supersede bởi [ADR-0073](0073-required-database-and-device-admission.md). OTA luôn kiểm tra Device; enrollment vẫn optional. Pending OTA contract và Unknown WS policy được mở rộng bởi [ADR-0074](0074-websocket-enrollment-session.md).

## Decision

Enrollment của Device chưa đăng ký là dữ liệu control-plane tạm thời trong SQLite hiện có. Trong mode ota, OTA khi feature bật chỉ trả activation code/challenge cho Device unknown; không trả websocket URL hoặc voice token. Trong mode websocket, trả transport config và omit activation; WS riêng hiển thị/phát mã trước claim theo ADR-0074. Admin claim chạy transaction `BEGIN IMMEDIATE`, tạo đúng một Device enabled, consume enrollment và ghi audit cùng transaction. Activation polling chỉ quan sát Device/enrollment: không cấp token, không consume code và không tải provider.

Enrollment yêu cầu database-backed admission và cấm auto-register. TTL được kiểm tra khi cấp/claim/poll nên correctness không phụ thuộc cleaner; cleaner chỉ expire/purge theo batch. Device/Agent disabled fail closed. Voice Session vẫn là boundary duy nhất resolve profile và acquire runtime.

## Consequences

Không thêm Redis, credential riêng hay database query vào SessionActor/audio path. Reverse proxy chịu rate limiting. Web UI phải gọi Admin claim qua same-origin Admin API và không suy online từ enrollment claim.
