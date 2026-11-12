# 10: Bỏ `admin_token`, bắt buộc `jwt_secret`

Status: resolved

**What to build:** Bước *contract*. Admin API chỉ còn nhận Admin Web Token (hoặc cho qua khi Password Login tắt). `admin_token` biến mất khỏi config, mã nguồn, test, script và tài liệu. Server từ chối khởi động khi Admin API bật mà thiếu `jwt_secret`, hoặc khi config còn khoá `admin_token`.

Từ ticket này cờ phải đổi mật khẩu chặn mọi request, vì không còn đường `admin_token` đi vòng.

Kết thúc ticket: chuyển [ADR 0141](../../../docs/adr/0141-admin-password-login-and-signed-web-tokens.md) sang `Accepted` và sửa dòng trạng thái của ADR 0051 thành "superseded in part by ADR 0141".

Xem spec: [Config](../spec.md#config), [Tài liệu](../spec.md#tài-liệu).

**Blocked by:** 04, 06, 07, 08, 09

**Status:** ready-for-agent

- [ ] Giá trị `admin_token` cũ gửi làm Bearer bị `401`
- [ ] `api.enabled = true` mà thiếu `jwt_secret` hoặc `jwt_secret` ngắn hơn 32 ký tự làm khởi động thất bại
- [ ] Config còn khoá `admin_token` làm khởi động thất bại với thông báo nói rõ phải thay bằng `jwt_secret`; giá trị cũ không được tự chuyển sang
- [ ] `jwt_secret` ghi đè được bằng biến môi trường theo cách `admin_token` từng được ghi đè
- [ ] Các kiểm tra từng so với `admin_token` (ví dụ ở bộ kiểm token System MCP) nay so với `jwt_secret`
- [ ] Không còn chỗ nào trong mã nguồn server, test, Admin Web, script hay config mẫu nhắc tới `admin_token`, trừ thông báo lỗi khởi động và ghi chú lịch sử trong ADR
- [ ] Script khởi tạo production sinh `jwt_secret` ngẫu nhiên; config mẫu, config Docker mẫu và config test của Docker đổi theo
- [ ] Tài liệu Admin API, Postman collection, README gốc, README và tài liệu của Admin Web mô tả luồng đăng nhập thay cho `admin_token`
- [ ] Có ghi chú nâng cấp nêu: thêm `jwt_secret`, xoá `admin_token`, script phải đăng nhập, Admin sẽ gặp mật khẩu mặc định
- [ ] ADR 0141 ở trạng thái `Accepted`; ADR 0051 ghi "superseded in part by ADR 0141"
- [ ] `scripts/agent-test.sh full`, `web test` và `web typecheck` đều qua
