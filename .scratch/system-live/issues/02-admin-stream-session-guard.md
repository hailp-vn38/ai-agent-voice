# 02: Stream SSE của Admin kết thúc khi token không còn được chấp nhận

**What to build:** Một helper trong `app::admin` bọc stream SSE của route. Helper kết thúc stream khi điều kiện mà middleware Admin dùng để cho request qua không còn đúng. Áp dụng cho mọi route SSE Admin. Sau ticket này, token hết hạn, "đăng xuất mọi nơi" hay đổi mật khẩu đều dừng các stream đang mở.

**Blocked by:** 01 (để áp cho `/system/live`; phần áp cho các stream cũ làm độc lập được).

Status: resolved

Spec: [system-live](../spec.md) · [ADR 0146](../../../docs/adr/0146-system-live-snapshot-stream.md) · [ADR 0141](../../../docs/adr/0141-admin-password-login-and-signed-web-tokens.md)

- [x] Helper giữ bearer của request và kiểm tra lại mỗi 5 giây: Password Login đang tắt, hoặc `AdminAuthState::accepts(bearer)` đúng. Điều kiện sai thì stream kết thúc không gửi thêm event.
- [x] Token có `exp`: hẹn giờ kết thúc đúng tại `exp`.
- [x] Không đọc SQLite trong lúc kiểm tra lại; không log bearer.
- [x] Áp dụng cho `/jobs/stream`, `/notifications/stream`, `/system/logs/stream`, `/knowledge/events`, `/system/live`. Không áp dụng cho `/knowledge/ask`.
- [x] Test đầy đủ helper qua một route: token hết hạn trong lúc stream mở; tăng revision qua `logout-everywhere`; bật Password Login khi stream được mở lúc Password Login đang tắt; tắt Password Login thì stream có token vẫn chạy.
- [x] Mỗi route còn lại có một test khói: tăng revision → stream kết thúc trong 5 giây (tokio time paused nếu được).
- [x] Client hiện có (`job-updates.ts`, `notifications.ts`, `system-logs.ts`, `knowledge-updates.ts`) vẫn đưa Admin về đăng nhập khi nối lại nhận 401; thêm test cho client nào chưa có.
- [x] ADR 0141 đã được kiểm tra; ADR 0146 đã ghi đủ chi tiết triển khai nên không cần thay đổi thêm.
- [x] Chạy test qua `scripts/agent-test.sh`.

## Comments

### Implemented on 2026-11-13

- Gom năm Admin SSE endpoint vào `admin_stream_routes` với một body guard. Guard giữ bearer, dùng điều kiện xác thực trong bộ nhớ, kiểm tra tối đa mỗi 5 giây và đặt deadline theo `exp`; không đọc SQLite hoặc ghi log token. `/knowledge/ask` không qua guard này.
- Test qua `/api/admin/system/live` cho token hết hạn thật, đổi revision, bật Password Login sau khi mở stream không bearer, và giữ stream có token khi Password Login bị tắt. Thêm fake-time test cho deadline hết hạn.
- Smoke test qua router thật cho jobs, notifications, system logs và knowledge events: đổi revision kết thúc từng stream trong một chu kỳ kiểm tra.
- Thêm test client notifications xác nhận HTTP 401 được chuyển tới `handleSessionResponse`.
- Khi thêm test route, phát hiện lần đọc `watch` đầu có thể lặp lại snapshot chưa đổi; đánh dấu receiver đã đọc và bỏ giá trị trùng.
- Gates: `scripts/agent-test.sh check vela-server` PASS; targeted server tests PASS (System Live 4 tests; route matrix 1 test); targeted notifications web test PASS (7 tests); `cargo fmt --all -- --check` PASS.
