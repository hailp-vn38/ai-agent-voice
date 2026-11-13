# 07: Trạng thái đồng bộ `wake_on_due` cho Admin

**What to build:** Reminder đang chờ hiển thị `wake_on_due` thay cho `pending` khi Device Push bật, kèm nhãn trên giao diện quản trị.

**Blocked by:** 01: OTA cấp thông tin Device Push Channel cho Device đã đăng ký.

Status: resolved

Spec: [device-push-wake](../spec.md) · [ADR 0145](../../../docs/adr/0145-device-push-channel-mqtt-wake-signal.md)

- [x] Admin API trả `wake_on_due` cho Reminder đang chờ mà lẽ ra là `pending`, khi tính năng bật.
- [x] `acked`, `out_of_window`, `unsupported` giữ nguyên nghĩa và độ ưu tiên; tính năng tắt thì vẫn trả `pending`.
- [x] Danh sách theo Device và theo Agent cho cùng kết quả.
- [x] Giao diện quản trị hiển thị nhãn tiếng Việt cho giá trị mới và không vỡ với giá trị lạ.
- [x] Cập nhật tài liệu API nếu có liệt kê các giá trị trạng thái đồng bộ.
- [x] Test theo mẫu test Reminder của Admin API hiện có.

## Comments

Resolved: sync_status reports wake_on_due for waiting reminders when [device_push] is enabled (create, device list, agent list); admin-web shows a Vietnamese label and falls back to the generic label for unknown values. No API doc enumerates sync values, so none changed.
