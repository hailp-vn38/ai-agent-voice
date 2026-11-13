# 02: Stream SSE của Admin kết thúc khi token không còn được chấp nhận

**What to build:** Một helper trong `app::admin` bọc stream SSE của route. Helper kết thúc stream khi điều kiện mà middleware Admin dùng để cho request qua không còn đúng. Áp dụng cho mọi route SSE Admin. Sau ticket này, token hết hạn, "đăng xuất mọi nơi" hay đổi mật khẩu đều dừng các stream đang mở.

**Blocked by:** 01 (để áp cho `/system/live`; phần áp cho các stream cũ làm độc lập được).

Status: ready-for-agent

Spec: [system-live](../spec.md) · [ADR 0146](../../../docs/adr/0146-system-live-snapshot-stream.md) · [ADR 0141](../../../docs/adr/0141-admin-password-login-and-signed-web-tokens.md)

- [ ] Helper giữ bearer của request và kiểm tra lại mỗi 5 giây: Password Login đang tắt, hoặc `AdminAuthState::accepts(bearer)` đúng. Điều kiện sai thì stream kết thúc không gửi thêm event.
- [ ] Token có `exp`: hẹn giờ kết thúc đúng tại `exp`.
- [ ] Không đọc SQLite trong lúc kiểm tra lại; không log bearer.
- [ ] Áp dụng cho `/jobs/stream`, `/notifications/stream`, `/system/logs/stream`, `/knowledge/events`, `/system/live`. Không áp dụng cho `/knowledge/ask`.
- [ ] Test đầy đủ helper qua một route: token hết hạn trong lúc stream mở; tăng revision qua `logout-everywhere`; bật Password Login khi stream được mở lúc Password Login đang tắt; tắt Password Login thì stream có token vẫn chạy.
- [ ] Mỗi route còn lại có một test khói: tăng revision → stream kết thúc trong 5 giây (tokio time paused nếu được).
- [ ] Client hiện có (`job-updates.ts`, `notifications.ts`, `system-logs.ts`, `knowledge-updates.ts`) vẫn đưa Admin về đăng nhập khi nối lại nhận 401; thêm test cho client nào chưa có.
- [ ] Cập nhật ADR 0141 nếu chi tiết triển khai khác với ADR 0146.
- [ ] Chạy test qua `scripts/agent-test.sh`.

## Comments
