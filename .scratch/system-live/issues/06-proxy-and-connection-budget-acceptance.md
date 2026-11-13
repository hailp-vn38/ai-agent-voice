# 06: Kiểm tra System Live Stream qua nginx của compose và ngân sách kết nối

**What to build:** Xác nhận stream hoạt động đúng qua `docker/nginx.conf` (không bị buffer, không bị cắt giữa chừng, đóng gọn khi server tắt) và nhiều tab không làm cạn ngân sách 6 kết nối mỗi origin trên HTTP/1.1.

**Blocked by:** 02, 04.

Status: ready-for-human

Spec: [system-live](../spec.md) · [ADR 0146](../../../docs/adr/0146-system-live-snapshot-stream.md)

- [ ] Chạy `docker/smoke.mjs` qua nginx và xác nhận event `snapshot` đầu tiên trong 2 giây, rồi event thứ hai trong 7 giây.
- [x] Đã kiểm tra location `/api` hiện có (`proxy_buffering off`, `proxy_read_timeout 3600s`); không cần sửa `nginx.conf`.
- [ ] Chạy `scripts/test-docker.sh` để dừng server khi client stream đang mở và xác nhận body kết thúc.
- [ ] Thủ công: mở 4 tab Admin, 3 tab ẩn → chỉ tab đang hiện giữ System Live Stream; các tab khác vẫn tải trang được.
- [ ] Ghi kết quả vào `## Comments`, kèm ghi chú nếu deployment thật đi qua proxy TLS có HTTP/2 (ví dụ `tailscale serve`).

## Comments

### Implemented on 2026-11-13

- Smoke qua Nginx đã được thêm vào `docker/smoke.mjs`: yêu cầu `text/event-stream`, xác thực Admin, `no-store`, `v: 1`, event đầu trong 2 giây và event kế tiếp trong 7 giây.
- `scripts/test-docker.sh` giữ kết nối mở qua Nginx, đợi cả hai snapshot rồi dừng container server; smoke client yêu cầu stream kết thúc trong 35 giây.
- Đã xác nhận cấu hình location `/api` hiện có `proxy_buffering off` và `proxy_read_timeout 3600s`; chưa thay đổi `nginx.conf`.
- Kết quả chạy Docker smoke: `NOT_RUN` trong worktree ticket vì web test image cần Dockerfile ở integration checkout; sẽ ghi kết quả thực tế sau khi chạy từ integration.
- Kiểm tra thủ công 4 tab và kiểm tra deployment qua proxy TLS/HTTP/2: `NOT_RUN`; repository không có cấu hình proxy TLS bên ngoài để xác nhận.
