# 06: Lệnh reset Admin Password và backup không mang trạng thái đăng nhập

Status: resolved

**What to build:** Admin quên mật khẩu chạy một lệnh con của binary server để đưa Admin Password về mặc định; lần đăng nhập sau lại bị buộc đổi mật khẩu. Ngoài ra, export rồi import backup không bao giờ thay đổi việc ai đăng nhập được vào server đích.

Xem spec: [Lệnh reset](../spec.md#lệnh-reset), [Backup](../spec.md#backup).

**Blocked by:** 02

**Status:** ready-for-agent

- [ ] Lệnh reset đặt mật khẩu về mặc định, bật Password Login, bật cờ phải đổi mật khẩu và tăng revision
- [ ] Sau reset và khởi động lại: token cũ bị `401`, mật khẩu cũ không đăng nhập được, mật khẩu mặc định đăng nhập được và bị buộc đổi
- [ ] Lệnh in ra rằng server đang chạy cần khởi động lại, và không in mật khẩu ngoài giá trị mặc định đã công khai
- [ ] Reset tạo một Admin Audit Event không chứa mật khẩu
- [ ] Database Backup Bundle export ra không chứa trạng thái đăng nhập
- [ ] Import từ bundle và từ Database Snapshot đều giữ nguyên Admin Password, Password Login và revision của server đích; token đang dùng vẫn hợp lệ sau import
- [ ] Test backup mới đặt cạnh các test backup hiện có
