# 04: Overview, System, Monitor, Runtime dùng `useSystemLive()`

**What to build:** Bốn trang đọc dữ liệu từ `useSystemLive()` thay cho polling. Sau ticket này các trang không còn `setInterval` lấy resources/status, biểu đồ CPU giữ lịch sử khi chuyển trang, và mỗi phần hiển thị trạng thái cũ hoặc không có dữ liệu một cách nhất quán.

**Blocked by:** 03.

Status: resolved

Spec: [system-live](../spec.md) · [ADR 0146](../../../docs/adr/0146-system-live-snapshot-stream.md)

- [x] `OverviewView.vue`, `SystemView.vue`, `SystemMonitorPage.vue`, `SystemRuntimePage.vue` bỏ các timer resources/status và xử lý `visibilitychange` riêng; đồng hồ hiển thị vẫn ở Overview.
- [x] Lịch sử CPU riêng ở Overview và Monitor được thay bằng `cpuHistory`; mỗi trang tự cắt số điểm cần vẽ.
- [x] Mỗi phần có chỉ báo khi `freshness` là `stale` hoặc `unavailable`; dùng chung `SystemFreshnessNotice` và thêm chuỗi i18n tiếng Anh/Việt.
- [x] Nút làm mới buộc nối lại khi `reconnecting`, ẩn khi `live`.
- [x] `SystemTransportPage.vue` giữ nguyên các probe `/health` và `/ready`.
- [x] Store `server` vẫn giữ `status` vì `SystemTransportPage` còn dùng; `checkHealth`/`refreshStatus` phục vụ trang Transport.
- [x] Test component của Overview, System, Monitor và Runtime dùng `useSystemLive` giả; bỏ các test polling cũ.
- [x] Kiểm tra trong trình duyệt: chuyển Overview → Monitor → Overview chỉ có một request `/system/live`; ẩn tab thì kết nối đóng; tab Network không còn request `/resources` định kỳ.
- [x] Chạy test qua `scripts/agent-test.sh`.

## Comments

### 2026-11-13

- Chuyển bốn trang sang snapshot composable và thêm nhãn độ tươi dùng chung; `SystemTransportPage` không đổi.
- `scripts/agent-test.sh run -- npm --prefix apps/admin-web run test -- --reporter=dot src/views/SystemView.test.ts` — 1 file, 19 test PASS.
- `scripts/agent-test.sh web typecheck` — PASS.
- `scripts/agent-test.sh web test` — 112 file, 682 test PASS.
- Kiểm tra trình duyệt thủ công **NOT_RUN**: worktree không có phiên Vela server tương thích để xác nhận số request SSE và hành vi tab ẩn; cần nghiệm thu thủ công.

### 2026-11-13 (review)

- Review: sau 401 composable để `connection = 'paused'` (không còn `reconnecting`), nên trang không hiện nút nối lại vô tác dụng.
- Overview khôi phục nút làm mới số liệu Admin (`admin.refreshAll()`); nút luôn hiện khi có Admin access, chỉ buộc nối lại System Live khi đang `reconnecting`, không gọi REST System.
- Kiểm tra trình duyệt qua Nginx của `compose.test.yaml` (cổng mở tạm), đăng nhập bằng Admin Password test:
  - Overview → Monitor → Overview → System → Runtime: đúng một `GET /api/admin/system/live`, không có `/system/resources`, `/api/admin/system`, `/health`, `/ready`.
  - Mô phỏng `visibilitychange` ẩn: request bị abort (`ERR_ABORTED`); hiện lại: mở ngay request mới.
  - `POST /auth/logout-all` từ phiên khác: trình duyệt về `/login?redirect=/overview`.
