# 07: Chuyển test Admin API sang Admin Web Token (lô 1)

**What to build:** Bước *migrate* thứ nhất. Các bộ test tích hợp gọi Admin API nhiều nhất thôi gửi `admin_token` và lấy Bearer từ helper của harness (ticket 02). `admin_token` vẫn còn trong config của test để CI xanh; ticket 10 mới xoá nó.

Phạm vi lô này: bộ test hợp đồng Admin API, bộ test memory admin, bộ test transcript archive.

Thay đổi là cơ học: thêm `jwt_secret` vào config của test và đổi chỗ lấy Bearer. Phần khẳng định của test không đổi.

**Blocked by:** 02

**Status:** ready-for-agent

- [ ] Không test nào trong ba bộ còn gửi giá trị `admin_token` làm Bearer
- [ ] Config của các test này có `jwt_secret` hợp lệ
- [ ] Không kỳ vọng hành vi nào của test bị sửa, trừ các test kiểm chính việc xác thực (sai token, thiếu token) được viết lại theo Admin Web Token
- [ ] Ba bộ test qua bằng `scripts/agent-test.sh targeted`
