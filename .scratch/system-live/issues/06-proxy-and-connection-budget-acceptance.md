# 06: Kiểm tra System Live Stream qua nginx của compose và ngân sách kết nối

**What to build:** Xác nhận stream hoạt động đúng qua `docker/nginx.conf` (không bị buffer, không bị cắt giữa chừng, đóng gọn khi server tắt) và nhiều tab không làm cạn ngân sách 6 kết nối mỗi origin trên HTTP/1.1.

**Blocked by:** 02, 04.

Status: ready-for-agent

Spec: [system-live](../spec.md) · [ADR 0146](../../../docs/adr/0146-system-live-snapshot-stream.md)

- [ ] `docker/smoke.mjs` (hoặc script smoke tương đương) mở `/api/admin/system/live` qua nginx và nhận event `snapshot` đầu tiên trong 2 giây, rồi event thứ hai trong 7 giây.
- [ ] Xác nhận location `/api` hiện có (`proxy_buffering off`, `proxy_read_timeout 3600s`) đủ dùng; chỉ sửa `nginx.conf` nếu smoke thất bại.
- [ ] Dừng container server trong lúc stream mở → client nhận stream kết thúc, không treo.
- [ ] Thủ công: mở 4 tab Admin, 3 tab ẩn → chỉ tab đang hiện giữ System Live Stream; các tab khác vẫn tải trang được.
- [ ] Ghi kết quả vào `## Comments`, kèm ghi chú nếu deployment thật đi qua proxy TLS có HTTP/2 (ví dụ `tailscale serve`).

## Comments
