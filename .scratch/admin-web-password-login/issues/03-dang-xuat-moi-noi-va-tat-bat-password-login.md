# 03: Đăng xuất mọi nơi và tắt/bật Password Login

**What to build:** Admin thu hồi được mọi Admin Web Token mà không đổi mật khẩu, và tắt hoặc bật lại Password Login. Khi Password Login tắt, Admin API cho qua request không có credential; bật lại thì đòi token như trước và mật khẩu cũ vẫn đăng nhập được. Phần này chỉ là API; giao diện nằm ở ticket 05.

Xem spec: [Route mới](../spec.md#route-mới), [Xác thực Admin API](../spec.md#xác-thực-admin-api).

**Blocked by:** 02

**Status:** ready-for-agent

- [ ] "Đăng xuất mọi nơi" trả token mới cho bên gọi; token cấp trước đó bị `401`, token mới dùng được
- [ ] Tắt Password Login đòi Admin Password hiện tại; sai mật khẩu bị từ chối
- [ ] Không tắt được Password Login khi cờ phải đổi mật khẩu còn bật
- [ ] Khi Password Login tắt: request không có header `Authorization` được chấp nhận, route trạng thái báo đã tắt, route đăng nhập trả `409 password_login_disabled`
- [ ] Khi Password Login tắt, bật lại không cần credential nhưng vẫn cần Admin Password hiện tại
- [ ] Sau khi bật lại: request không có credential bị `401` và mật khẩu cũ đăng nhập được
- [ ] Request có nhiều hơn một header `Authorization` vẫn bị từ chối ở mọi trạng thái
- [ ] Mỗi thao tác thành công tạo đúng một Admin Audit Event không chứa mật khẩu
