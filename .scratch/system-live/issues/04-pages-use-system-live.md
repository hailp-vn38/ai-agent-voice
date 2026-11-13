# 04: Overview, System, Monitor, Runtime dùng `useSystemLive()`

**What to build:** Bốn trang đọc dữ liệu từ `useSystemLive()` thay cho polling. Sau ticket này các trang không còn `setInterval` lấy resources/status, biểu đồ CPU giữ lịch sử khi chuyển trang, và mỗi phần hiển thị trạng thái cũ hoặc không có dữ liệu một cách nhất quán.

**Blocked by:** 03.

Status: ready-for-agent

Spec: [system-live](../spec.md) · [ADR 0146](../../../docs/adr/0146-system-live-snapshot-stream.md)

- [ ] `OverviewView.vue`, `SystemView.vue`, `SystemMonitorPage.vue`, `SystemRuntimePage.vue` bỏ các timer resources/status và xử lý `visibilitychange` riêng; đồng hồ hiển thị ("x giây trước") vẫn ở trang.
- [ ] Lịch sử CPU riêng ở Overview và Monitor được thay bằng `cpuHistory`; mỗi trang tự cắt số điểm cần vẽ.
- [ ] Mỗi phần có chỉ báo khi `freshness` là `stale` hoặc `unavailable`; dùng chung một component nhỏ nếu có từ hai chỗ dùng trở lên. Thêm chuỗi i18n cho mọi ngôn ngữ đang có.
- [ ] Nút làm mới: buộc nối lại khi `reconnecting`, ẩn khi `live`.
- [ ] `SystemTransportPage.vue` giữ nguyên các probe `/health` và `/ready`.
- [ ] Store `server`: bỏ `status` nếu không còn nơi nào khác dùng; giữ `checkHealth`/`refreshStatus` cho trang Transport.
- [ ] Test component của bốn trang dùng `useSystemLive` giả; xoá test về timer polling cũ.
- [ ] Kiểm tra thủ công trong trình duyệt: chuyển Overview → Monitor → Overview chỉ có một request `/system/live`; ẩn tab thì kết nối đóng; tab Network không còn request `/resources` định kỳ.
- [ ] Chạy test qua `scripts/agent-test.sh`.

## Comments
