# 06: Kiểm tra System Live Stream qua nginx của compose và ngân sách kết nối

**What to build:** Xác nhận stream hoạt động đúng qua `docker/nginx.conf` (không bị buffer, không bị cắt giữa chừng, đóng gọn khi server tắt) và nhiều tab không làm cạn ngân sách 6 kết nối mỗi origin trên HTTP/1.1.

**Blocked by:** 02, 04.

Status: ready-for-human

Spec: [system-live](../spec.md) · [ADR 0146](../../../docs/adr/0146-system-live-snapshot-stream.md)

- [x] Chạy `docker/smoke.mjs` qua nginx và xác nhận event `snapshot` đầu tiên trong 2 giây, rồi event thứ hai trong 7 giây.
- [x] Xác nhận runtime qua location `/api` hiện có (`proxy_buffering off`, `proxy_read_timeout 3600s`); chưa cần sửa `nginx.conf` khi smoke chưa chạy được.
- [x] Chạy `scripts/test-docker.sh` để dừng server khi client stream đang mở và xác nhận body kết thúc.
- [ ] Thủ công: mở 4 tab Admin, 3 tab ẩn → chỉ tab đang hiện giữ System Live Stream; các tab khác vẫn tải trang được.
- [x] Ghi kết quả vào `## Comments`, kèm ghi chú nếu deployment thật đi qua proxy TLS có HTTP/2 (ví dụ `tailscale serve`).

## Comments

### Implemented on 2026-11-13

- Smoke qua Nginx đã được thêm vào `docker/smoke.mjs`: yêu cầu `text/event-stream`, xác thực Admin, `no-store`, `v: 1`, event đầu trong 2 giây và event kế tiếp trong 7 giây.
- `scripts/test-docker.sh` giữ kết nối mở qua Nginx, đợi cả hai snapshot rồi dừng container server; smoke client yêu cầu stream kết thúc trong 35 giây.
- Đã xác nhận cấu hình location `/api` hiện có `proxy_buffering off` và `proxy_read_timeout 3600s`; chưa thay đổi `nginx.conf`.
- Docker gate lần đầu dừng trước khi chạy `server-check` vì bind path mặc định `/mnt/storage/ai-agent-voice/docker/test-target` không tồn tại. Chạy lại với `VOICE_TEST_TARGET_DIR=/tmp/system-live-docker-target` thì `server-check` dừng ở Clippy: có lint `new_without_default` trong `SystemMetrics`, `explicit_counter_loop` trong auth route test và `too_many_arguments` có sẵn ở `providers/llm/gateway/wire_gemini.rs` (file này không thuộc diff).
- `scripts/agent-test.sh run -- docker compose -p vela-test -f compose.test.yaml --profile checks run --rm --no-deps web-check`: PASS, 112 test files.
- Runtime `docker compose up --detach --wait server web` chưa lên được: container `vela-server:qualification` thoát với `Read-only file system` trong startup dù mount `/app/data` báo RW; Nginx snapshot và shutdown smoke vì vậy `NOT_RUN`.
- Kiểm tra thủ công 4 tab và kiểm tra deployment qua proxy TLS/HTTP/2: `NOT_RUN`; repository không có cấu hình proxy TLS bên ngoài để xác nhận.

### 2026-11-13 (review, gates chạy lại)

- Nguyên nhân container server thoát với `Read-only file system`: built-in job skills được cài vào `skills/` (mặc định `skills_dir`), nằm ngoài volume `/app/data`. CAM++ chỉ là cảnh báo không chặn. Sửa `docker/config.test.toml`: `[orchestrator] skills_dir = "data/skills"`.
- `docker/smoke.mjs` vẫn dùng admin token tĩnh `docker-test-admin` đã bị bỏ ở ADR 0141, nên mọi lần đọc Admin đều 401. Smoke giờ đăng nhập bằng Admin Password mặc định, đổi sang Password test ở lần đầu và dùng lại Password test sau restart.
- Clippy `too_many_arguments` có sẵn ở `wire_gemini.rs` được `allow` như các wire adapter khác; `cargo clippy -p vela-server --all-targets -D warnings` PASS.
- PASS: `up --wait server web`; `smoke` (event `snapshot` đầu ≤ 2s, event kế ≤ 7s qua Nginx); `restart server` + `smoke after-restart`; `system-live-shutdown` (stream qua Nginx kết thúc khi dừng server).
- Ngân sách kết nối: tab ẩn đóng System Live Stream (đã kiểm bằng mô phỏng `visibilitychange` trong một tab).
- 4 tab trong browser pane của Claude desktop: pane không đánh dấu tab nền là `hidden`, nên đây là trường hợp xấu nhất "4 tab cùng hiện". Mỗi tab giữ notifications stream và System Live Stream (8 kết nối qua HTTP/1.1), và tab thứ tư không hoàn thành nổi `GET /api/admin/auth/status` trong 8 giây. Kết quả khớp phần Consequences của ADR 0146. Với 3 tab thật sự ẩn thì chỉ còn 4 notifications + 1 System Live = 5 kết nối, nhưng mục thủ công này vẫn cần một trình duyệt thật để xác nhận.
- Nếu Admin thường mở nhiều tab cùng hiện, biện pháp triệt để vẫn là HTTP/2 ở proxy TLS phía trước, hoặc chia sẻ kết nối giữa các tab (đang Out of Scope).
- `server-check` (toàn bộ `cargo test` trong Docker) chưa chạy trong lần này; thay bằng `scripts/agent-test.sh package vela-server` cục bộ.
- Deployment qua proxy TLS/HTTP/2 (ví dụ `tailscale serve`): repository không có cấu hình để kiểm; HTTP/2 vẫn là biện pháp triệt để theo ADR 0146.
