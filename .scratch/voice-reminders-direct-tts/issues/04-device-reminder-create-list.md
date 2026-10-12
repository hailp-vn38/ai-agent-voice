# 04: Tạo và xem Reminder bền vững ở Device Detail

**What to build:** Admin lưu một Reminder cho physical Device và xem lại sau server restart, với trạng thái saved/chưa sync trung thực.

**Blocked by:** None (can start immediately).

Status: ready-for-agent

- [ ] Forward-only migration và DeviceNotifications giữ Reminder theo Device, Agent grant default deny có Admin GET/PATCH + revision và setting UI để operator cho phép delivery, Schedule Revision/row revision tách biệt; bảo toàn dữ liệu cũ.
- [ ] Admin authenticated create/list có filter/page, quota tối đa 500 pending, request identity/error/audit conventions và UI form/list loading/error/i18n/accessibility.
- [ ] Server clock tính relative 1–31.536.000 giây hoặc absolute RFC3339/timezone; đúng một time input, validate text/controls/bytes, giờ mơ hồ không đoán.
- [ ] Create + Schedule Revision + durable mutation key/response atomic; retry cùng payload replay sau concurrency/restart, khác payload conflict, không tăng revision khi replay/rollback.
- [ ] Device owner không đổi khi reassignment; Virtual Device không được chọn làm physical wake target; enabled không được trình bày là WS online.
- [ ] Public HTTP tests/fake clock/Web tests chứng minh persist/restart, auth/validation/quota và sync pending/unsupported; chưa có scheduler không được báo reminder đã phát.
