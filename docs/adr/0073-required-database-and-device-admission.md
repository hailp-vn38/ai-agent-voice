# ADR 0073 — Database và Device admission luôn bật

## Status

Accepted — theo quyết định của người dùng; supersedes các clause opt-in trong ADR-0049,
ADR-0057 và nhánh legacy OTA trong ADR-0072.

## Decision

SQLite là startup dependency bắt buộc. Xóa `database.enabled` và
`database.devices.admission_enabled` khỏi config/DTO/defaults; strict parser từ chối
cả hai key cũ thay vì silently ignore. Bỏ section database vẫn dùng SQLite defaults,
không đồng nghĩa tắt database. Validation URL/pool/timeout/retention luôn áp dụng.

Mọi public startup/router-building seam mở SQLite, kiểm tra schema và chạy migration
trước provider initialization/router publication. DB lỗi fail trước bind. Mọi WS
đi qua Database-backed Device Admission: unknown/disabled Device hoặc disabled Agent
trả 403, DB/profile/runtime unavailable trả 503 trước upgrade; không fallback vì DB
lỗi. Agent đã admit mà không có Template assignment vẫn có thể dùng server defaults
theo policy hiện tại; đó là fallback cấu hình cho một Device đã xác minh, không phải
bỏ qua admission.

Readiness luôn probe DB bằng SELECT 1, kể cả khi Admin API/history/enrollment tắt.
Injected AppState thiếu DB là startup_incomplete và không admit WS. Session đang chạy
giữ snapshot immutable khi DB lỗi; liveness vẫn độc lập dependency.

OTA luôn kiểm tra Device trước khi trả websocket/token. Registered nhận cấu hình;
blocked bị từ chối; unknown chỉ nhận activation nếu enrollment bật, nếu không trả
403. Poll/claim/cleaner giữ contract ADR-0072. OTA không auto-register hoặc load model.

Giữ các cờ `database.devices.enrollment.enabled`, `database.history.enabled`,
`api.enabled` và `database.devices.auto_register`. API chỉ phụ thuộc api.enabled
và bearer admin riêng; enrollment cần API bật và auto_register=false. Auto-register
chỉ là chế độ dev/migration đã có, không phải cơ chế bỏ qua DB.

## Consequences

Operator xóa hai key cũ, tạo parent directory cho SQLite và provision Device/Agent
trước voice session. DB trống vẫn khởi động được, không provision application rows
ngầm. Không cần schema migration mới vì đây là thay đổi policy/config, không đổi
bảng hoặc checksum SQLx. Test protocol dùng file-backed DB có Agent/Device thay
database-free router. Router/application convenience seams trở thành async và có
BootstrapError để DB failure được report trước publication.
