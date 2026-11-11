# 05: Trang Bảo mật trong System

**What to build:** Trong mục System có trang Bảo mật, nơi Admin đổi mật khẩu, đăng xuất, đăng xuất mọi nơi, và tắt hoặc bật lại Password Login. Tắt Password Login hiện hộp xác nhận cảnh báo rằng ai tới được server đều có toàn quyền, và đòi nhập mật khẩu hiện tại. Khi Password Login tắt, Admin Web mở thẳng vào app, và trang Bảo mật vẫn vào được để bật lại.

Xem spec: [Admin Web](../spec.md#admin-web).

**Blocked by:** 03, 04

**Status:** ready-for-agent

- [ ] Trang Bảo mật có trong điều hướng System và có tiêu đề ở cả hai ngôn ngữ
- [ ] Đổi mật khẩu từ trang này giữ Admin ở trạng thái đã đăng nhập bằng token mới
- [ ] "Đăng xuất mọi nơi" giữ trình duyệt hiện tại đăng nhập bằng token mới
- [ ] Tắt Password Login chỉ gửi request sau khi Admin xác nhận và nhập mật khẩu hiện tại; sai mật khẩu hiện lỗi và công tắc không đổi
- [ ] Khi Password Login tắt, mở Admin Web không qua trang đăng nhập
- [ ] Bật lại Password Login từ trang này đòi mật khẩu hiện tại, sau đó Admin Web yêu cầu đăng nhập
- [ ] Test Vitest phủ luồng xác nhận tắt và hiển thị lỗi; typecheck qua
