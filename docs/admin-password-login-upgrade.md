# Nâng cấp: Admin đăng nhập bằng Admin Password

Từ bản này Admin API không còn nhận `admin_token`. Credential duy nhất là Admin Web Token cấp sau khi đăng nhập bằng Admin Password, ký bằng `api.jwt_secret`. Xem [ADR 0141](adr/0141-admin-password-login-and-signed-web-tokens.md).

Thay đổi phá vỡ tương thích. Làm theo thứ tự trước khi khởi động bản mới:

1. **Thêm `jwt_secret`** vào `[api]`: chuỗi ngẫu nhiên tối thiểu 32 ký tự (ví dụ `python3 -c 'import secrets; print(secrets.token_urlsafe(32))'`), khác `[auth].token` và khoá `VOICE_SYSTEM_MCP_JWT_KEY`. Có thể đặt qua `VOICE_ADMIN_JWT_SECRET` thay cho `VOICE_ADMIN_TOKEN` cũ. Khi `api.enabled = true` mà thiếu hoặc ngắn hơn 32 ký tự, server không khởi động.
2. **Xoá `admin_token`** khỏi `[api]` và bỏ biến môi trường `VOICE_ADMIN_TOKEN`. Config còn khoá này làm server từ chối khởi động; giá trị cũ không được tự chuyển sang `jwt_secret`. Giá trị cũ gửi làm Bearer bị `401`.
3. **Script và CLI phải đăng nhập**: `POST /api/admin/auth/login` với `{"password": "..."}`, lấy `token` trong response, rồi gửi `Authorization: Bearer <token>`. Token có hạn (`api.token_ttl_secs`); đăng nhập lại khi hết hạn.
4. **Admin sẽ gặp mật khẩu mặc định**: database chưa có Admin Password dùng mật khẩu `admin` và cờ buộc đổi mật khẩu. Token đăng nhập bằng mật khẩu mặc định chỉ gọi được `POST /api/admin/auth/password` (các route khác trả `403 password_change_required`). Đổi mật khẩu ngay sau khi nâng cấp, nhất là trên máy mở ra mạng.

Đổi `jwt_secret` làm mọi Admin Web Token đang dùng mất hiệu lực. Postman collection mới có thư mục `00 - Admin login` tự lưu token vào biến `admin_web_token`. `scripts/prod-init.sh` sinh `jwt_secret` ngẫu nhiên cho `docker/config.prod.toml` mới.
