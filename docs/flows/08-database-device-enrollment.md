# Flow 08 — Database bắt buộc và thêm Device

## Cấu hình hiện hành

SQLite và WS Device admission luôn hoạt động. Xóa `enabled` trong `[database]` và
`admission_enabled` trong `[database.devices]` ở mọi deployment/test config. Hai
key cũ, kể cả giá trị true, bị strict parser từ chối. `[database]` có thể bỏ để dùng
default `sqlite://data/voice-agent.db`; chuẩn bị parent directory có quyền ghi trước boot.

```toml
[database]
url = "sqlite://data/voice-agent.db"
max_connections = 5
busy_timeout_ms = 5000
migrate_on_start = true

[database.devices]
auto_register = false

[database.devices.enrollment]
enabled = true
code_ttl_seconds = 600
retention_seconds = 86400
cleanup_interval_seconds = 60
max_pending = 1000
transport = "websocket"
prompt_assets_dir = "assets/enrollment/vi-VN"

[database.history]
enabled = false
retention_days = 30
queue_capacity = 256

[api]
enabled = true
admin_token = "<deployment-admin-token>"
```

Enrollment có thể tắt để chỉ thêm thủ công. API cũng có thể tắt, nhưng khi enrollment
bật API phải bật và auto_register phải false. Tắt ghi history không tắt DB, admission
hoặc cleanup archive. Default database trống không cấp quyền WS cho thiết bị nào.

## Startup và readiness

Validate config → mở SQLite/WAL/foreign keys → kiểm tra migration compatibility →
migrate hoặc xác nhận schema current → resolve/load provider runtimes → bind listener.
DB unavailable/schema incompatible/migration failure dừng trước bind, cả legacy
provider mode và managed runtime mode. `/health` liveness không đổi; `/ready` luôn
probe SQLite, không tải model hoặc resolve toàn graph. DB outage làm readiness 503
và admission mới 503; session đã admit vẫn dùng immutable profile.

## Thêm Device bằng mã

1. Chuẩn bị 11 WAV clips bằng `python3 scripts/prepare-enrollment-assets.py` hoặc
   recording riêng. Mode websocket default khi bật enrollment; mode ota giữ flow cũ.
2. Device gửi Device-Id/Client-Id tới OTA. Unknown mode websocket nhận URL/token
   transport và không activation. Mở WS → hello → hiển thị/phát mã sáu digit trong
   Enrollment Session riêng; không hội thoại/provider. Disabled Device/Agent 403.
3. Web chọn Agent, nhập mã 6 số dạng string và Template tùy chọn.
4. Admin claim dùng bearer riêng: transaction tạo Device, consume mã và audit.
   Runtime chưa được load ở claim. TTL/concurrent/replay behavior giữ ADR-0072.
5. WS worker thấy Registered, gửi thông báo và close 1000; client mở lại WS khi
   wake/bấm nút hoặc reconnect đã implement. Mode ota mới dùng vòng activate
   pending 202 → Registered 200, gọi lại OTA lấy WS config.
6. WS voice mới validate identity/auth → DB Device/Agent → Template/provider snapshot →
   runtime acquire → upgrade. Không có nhánh admission_disabled.

## Thêm thủ công và enrollment tắt

Bật Admin API, tạo Agent rồi `POST /api/admin/devices` với device_id/agent_key và
template_key nếu cần. Device phải tồn tại trước OTA; unknown khi enrollment tắt
trả 403, không cấp mã hoặc token. Device đã tạo thủ công đi qua cùng OTA/WS flow.
Admin tắt không làm các Device đã provision ngừng hoạt động; chỉ tắt management.

`auto_register=true` vẫn chỉ là chế độ dev/migration tại WS admission và cần Agent
enabled đã provision. OTA không auto-register, nên công cụ dùng OTA phải được
provision trước hoặc dùng enrollment. Không bật auto_register cùng enrollment.

## Web/API đồng bộ

- Modal Add Device chỉ phụ thuộc enrollment capability và API auth, không hiển thị
  lựa chọn bật database/admission. Giữ tab thêm thủ công.
- Admin router chỉ mount theo api.enabled, không theo cờ database đã bị xóa.
- Thiết bị đã liên kết, enabled, runtime ready và online là các trạng thái riêng.
- Null template_key vẫn dùng default/fallback policy của Agent đã admit.
- GET/system field database.enabled=true là capability cố định để tương thích API,
  không phải config toggle. Không cần thay đổi schema Device hoặc Postman request.
- Claim/list không tác động session đang chạy; rebind/re-enable áp dụng connection mới.

## Chuyển deployment cũ

1. Xóa hai key cũ trong config file, generated config, test fixtures và tài liệu UI.
2. Giữ đường dẫn SQLite hiện tại; không xóa DB hoặc reset SQLx migration history.
3. Tạo parent directory nếu chưa có; giữ backup/forward-only policy hiện tại.
4. Provision mọi Device đang sử dụng trước restart. Config cũ từng tắt admission
   sẽ thấy Device unknown bị từ chối sau nâng cấp; không tự cấp Device/Agent ngầm.
5. Kiểm tra readiness, OTA unknown/blocked/registered, claim, poll rồi WS.
6. Khi rollback config, dùng phiên bản cấu hình tương ứng binary; không sửa schema
   để né compatibility guard. Thay đổi này không thêm migration SQL.

Xem [ADR-0073](../adr/0073-required-database-and-device-admission.md),
[ADR-0074](../adr/0074-websocket-enrollment-session.md) và
[WS enrollment](../device-enrollment-websocket.md).
