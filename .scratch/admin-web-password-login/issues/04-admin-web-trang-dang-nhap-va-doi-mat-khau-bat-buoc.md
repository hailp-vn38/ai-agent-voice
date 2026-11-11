# 04: Admin Web có trang đăng nhập và đổi mật khẩu bắt buộc

**What to build:** Mở Admin Web khi chưa có Admin Web Token, Admin thấy trang đăng nhập thay cho ô dán token. Đăng nhập bằng mật khẩu mặc định đưa thẳng tới trang đổi mật khẩu; đổi xong mới vào được app và quay về trang đã định đến. Token hết hạn hoặc bị thu hồi thì Admin được đưa về trang đăng nhập. Bundle không còn chứa credential nào.

Gồm cả việc kiểm tra Studio Assistant và các trang streaming (job, log, thông báo, wiki chat) vẫn chạy sau khi đăng nhập bằng token mới.

Xem spec: [Admin Web](../spec.md#admin-web), [Seam 2](../spec.md#seam-2-admin-web-đã-có).

**Blocked by:** 02

**Status:** ready-for-agent

- [ ] Ô dán token và biến môi trường nhúng token vào bundle bị xoá, kể cả trong Dockerfile, file env mẫu và khai báo kiểu
- [ ] Khởi động đọc trạng thái đăng nhập trước khi tải dữ liệu và chọn đúng một trong ba: vào app, trang đăng nhập, trang đổi mật khẩu
- [ ] Guard của router chặn mọi route khi chưa có token, và chỉ cho vào trang đổi mật khẩu khi cờ phải đổi mật khẩu bật
- [ ] Sau khi đăng nhập hoặc đổi mật khẩu, Admin quay về route đã định đến
- [ ] Token lưu trong `localStorage`, dùng được ở tab mới và sau reload; hàm đọc token mà client HTTP và các luồng SSE gọi giữ nguyên chữ ký
- [ ] `401` ở bất kỳ request nào, kể cả luồng SSE tự mở bằng `fetch`, xoá token và đưa về trang đăng nhập
- [ ] `403 password_change_required` đưa về trang đổi mật khẩu
- [ ] Trang đăng nhập hiện thông báo riêng cho sai mật khẩu và cho quá số lần thử; trang đổi mật khẩu hiện thông báo riêng cho từng quy tắc bị vi phạm
- [ ] Nút đăng xuất xoá token ở trình duyệt này và về trang đăng nhập
- [ ] Mọi chuỗi mới có bản tiếng Anh và tiếng Việt
- [ ] Studio Assistant và các trang streaming chạy được với Admin Web Token; nếu Studio Assistant cần thay đổi ở server thì ghi lại trong phần Comments thay vì tự mở rộng phạm vi
- [ ] Test Vitest phủ client HTTP, guard và hai trang mới; typecheck qua
