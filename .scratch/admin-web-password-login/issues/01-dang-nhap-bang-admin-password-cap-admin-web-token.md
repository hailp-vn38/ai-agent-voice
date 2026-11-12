# 01: Đăng nhập bằng Admin Password cấp Admin Web Token

Status: resolved

**What to build:** Một client gọi route đăng nhập của Admin API với Admin Password mặc định và nhận về Admin Web Token; token đó được Admin API chấp nhận làm Bearer. Đây là bước *expand*: `admin_token` vẫn hoạt động y như cũ, nên không test hay script nào hiện có bị ảnh hưởng.

Gồm: migration thêm bản ghi singleton trạng thái đăng nhập, seed mật khẩu mặc định khi chưa có hash, khoá `api.jwt_secret` trong config (ở ticket này còn **tuỳ chọn**: thiếu thì hai route mới trả `404` và chỉ `admin_token` dùng được), route đọc trạng thái, route đăng nhập, limiter chống dò, và middleware xác thực nhận thêm Admin Web Token. Thêm ba thuật ngữ Admin Password, Admin Web Token, Admin Signing Secret vào `CONTEXT.md` và `GLOSSARY.md`.

Ticket này chưa chặn theo cờ phải đổi mật khẩu; token cấp từ mật khẩu mặc định gọi được mọi route. Ticket 02 thêm việc chặn.

Xem spec: [Mô hình](../spec.md#mô-hình), [Admin Web Token](../spec.md#admin-web-token), [Xác thực Admin API](../spec.md#xác-thực-admin-api), [Chống dò mật khẩu](../spec.md#chống-dò-mật-khẩu). Quyết định: [ADR 0141](../../../docs/adr/0141-admin-password-login-and-signed-web-tokens.md).

**Blocked by:** None (can start immediately).

**Status:** ready-for-agent

- [ ] Trên SQLite mới và config có `jwt_secret`, đăng nhập bằng `admin` trả token, thời điểm hết hạn và cờ phải đổi mật khẩu bật
- [ ] Token vừa cấp được một route Admin API bất kỳ chấp nhận
- [ ] Route đọc trạng thái không cần credential và chỉ trả Password Login bật/tắt cùng cờ phải đổi mật khẩu
- [ ] Mật khẩu sai trả `401 invalid_password`; sau 5 lần sai trong 60 giây trả `429 too_many_attempts` kèm `Retry-After`
- [ ] Token hết hạn bị `401` (test đặt thời hạn ngắn qua config, không can thiệp đồng hồ)
- [ ] Token bị sửa một ký tự, ký bằng secret khác, hoặc dùng thuật toán `none` đều bị `401`
- [ ] Dựng lại app trên cùng SQLite và cùng `jwt_secret`: token cũ còn dùng được; với `jwt_secret` khác: bị `401`
- [ ] Token System MCP không được Admin API chấp nhận và Admin Web Token không được endpoint System MCP chấp nhận
- [ ] Đăng nhập sai phát Structured Log Entry mức cảnh báo và không tạo Admin Audit Event
- [ ] Mật khẩu, hash, token và `jwt_secret` không xuất hiện trong log, `Debug` hay body lỗi
- [ ] `jwt_secret` có trong config nhưng ngắn hơn 32 ký tự, trùng `auth.token`, hoặc trùng khoá JWT của System MCP làm khởi động thất bại
- [ ] Toàn bộ test hiện có qua mà không sửa; `admin_token` vẫn được chấp nhận
