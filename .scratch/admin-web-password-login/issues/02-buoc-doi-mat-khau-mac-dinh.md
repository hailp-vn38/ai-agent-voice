# 02: Buộc đổi mật khẩu mặc định và đổi Admin Password

**What to build:** Sau khi đăng nhập bằng mật khẩu mặc định, Admin Web Token chỉ gọi được route đổi mật khẩu và route đọc trạng thái; mọi route khác trả `403 password_change_required`. Đổi mật khẩu thành công trả token mới dùng được ngay, và mọi token cấp trước đó hết hiệu lực. Admin đổi lại mật khẩu bất kỳ lúc nào bằng cùng route.

Kèm một helper trong harness test Admin API: đăng nhập bằng mật khẩu mặc định, đổi mật khẩu, trả Admin Web Token dùng được. Các ticket chuyển test (07–09) dựa vào helper này.

Trong giai đoạn expand, request bằng `admin_token` không bị cờ phải đổi mật khẩu chặn.

Xem spec: [Xác thực Admin API](../spec.md#xác-thực-admin-api), [Route mới](../spec.md#route-mới), [Audit và log](../spec.md#audit-và-log).

**Blocked by:** 01

**Status:** ready-for-agent

- [ ] Token cấp từ mật khẩu mặc định bị `403 password_change_required` ở một route bất kỳ, nhưng gọi được route đổi mật khẩu và route trạng thái
- [ ] Đổi mật khẩu trả token mới; token mới gọi được route trước đó bị chặn và cờ phải đổi mật khẩu tắt
- [ ] Token cấp trước khi đổi mật khẩu bị `401`
- [ ] Mật khẩu mới đăng nhập được; mật khẩu mặc định không còn đăng nhập được
- [ ] Sai mật khẩu hiện tại bị từ chối; mật khẩu mới ngắn hơn 8 ký tự, bằng mặc định, hoặc bằng mật khẩu hiện tại trả `422` với mã lỗi riêng từng trường hợp
- [ ] Mỗi lần đổi mật khẩu thành công tạo đúng một Admin Audit Event trong cùng transaction, không chứa mật khẩu, hash hay token
- [ ] Trạng thái giữ trong bộ nhớ khớp SQLite sau khi đổi: dựng lại app không làm thay đổi kết quả của các kiểm tra trên
- [ ] Harness có helper trả Admin Web Token dùng được, và có ít nhất một test hiện có dùng thử helper này
