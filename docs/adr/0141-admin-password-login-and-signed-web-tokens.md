# ADR 0141 — Admin Password Login và Admin Web Token ký bằng secret trong config

Status: Proposed, 2026-11-11.
Supersedes (phần xác thực): [ADR 0051](0051-separate-authenticated-admin-api.md).
Extends: [ADR 0058](0058-minimal-admin-mutation-audit.md), [ADR 0131](0131-database-backup-export-and-transactional-import.md).

Admin API hiện chỉ nhận một `admin_token` tĩnh trong config. Admin Web bắt Admin dán token đó vào trang; token không có hạn, không đổi được từ giao diện và có thể bị nhúng vào bundle. Spec: [admin-web-password-login](../../.scratch/admin-web-password-login/spec.md).

Quyết định: bỏ `admin_token`. Admin đăng nhập bằng **Admin Password**, server cấp **Admin Web Token** (JWT có hạn) ký bằng **Admin Signing Secret** trong config. Đây là credential duy nhất của Admin API, dùng chung cho Admin Web, script và CLI.

## Decisions

- **Một Admin, một mật khẩu.** Không có tài khoản, tên đăng nhập hay vai trò. Trạng thái đăng nhập là một bản ghi singleton trong SQLite: Password Login bật/tắt, hash và salt của Admin Password, cờ phải đổi mật khẩu, revision.
- **Mật khẩu mặc định và buộc đổi.** Deployment chưa có hash được seed mật khẩu mặc định `admin` với cờ phải đổi mật khẩu. Khi cờ bật, mọi route trừ đổi mật khẩu và đọc trạng thái trả `403 password_change_required`.
- **Hash bằng PBKDF2-HMAC-SHA256 có salt**, dùng thư viện mật mã đã có trong workspace. Không thêm dependency.
- **Admin Web Token là JWT HS256** với thời điểm cấp, thời điểm hết hạn và revision lúc cấp. Thời hạn mặc định 7 ngày, cấu hình được. Không có refresh token.
- **Thu hồi bằng revision, không bằng danh sách.** Đổi mật khẩu, reset mật khẩu và "đăng xuất mọi nơi" tăng revision; token mang revision cũ bị từ chối. Không thu hồi được từng token riêng lẻ.
- **Admin Signing Secret nằm trong config (`api.jwt_secret`), không nằm trong SQLite.** Bắt buộc dài ít nhất 32 ký tự khi `api.enabled = true`, nếu không server từ chối khởi động. Không được trùng `auth.token` hay khoá JWT của System MCP. Không log, không `Debug`.
- **`admin_token` bị bỏ, không có đường chuyển tiếp lúc chạy.** Config còn khoá `admin_token` làm khởi động thất bại với thông báo chỉ sang `jwt_secret`. Bearer tĩnh không còn được chấp nhận.
- **Password Login tắt được.** Khi tắt, Admin API cho qua mọi request không cần credential, ở mọi kiểu bind. Tắt và bật đều đòi Admin Password hiện tại; không tắt được khi còn phải đổi mật khẩu.
- **Limiter chống dò trong tiến trình cho riêng route đăng nhập**: 5 lần sai trong 60 giây, tính chung toàn server, giữ trong bộ nhớ.
- **Audit theo ADR 0058.** Đổi mật khẩu, đăng xuất mọi nơi, đặt Password Login và reset ghi Admin Audit Event trong cùng transaction. Đăng nhập sai chỉ ra telemetry.
- **Backup không mang trạng thái đăng nhập.** Database Backup Bundle không chứa nó; import từ bundle hay snapshot đều không áp nó vào server đích. Database Snapshot có chứa hash vì là bản sao nguyên file.
- **Khôi phục bằng lệnh con của binary server** đưa Admin Password về mặc định, bật Password Login, bật cờ phải đổi mật khẩu và tăng revision.
- **Admin Web giữ token trong `localStorage`** và tiếp tục gửi header Bearer. Bundle không chứa credential nào. Chính sách CORS của ADR 0051 giữ nguyên: không cookie credentials.

## Phần của ADR 0051 bị thay

| ADR 0051 | ADR này |
|---|---|
| Mọi request bắt buộc đúng một Bearer bằng `admin_token` | Bearer phải là Admin Web Token hợp lệ, hoặc Password Login đang tắt |
| `admin_token` trim-rỗng làm startup fail-fast | `jwt_secret` thiếu hoặc ngắn hơn 32 ký tự làm startup fail-fast |
| Không có limiter chống dò trong tiến trình | Có limiter cho route đăng nhập |
| Credential Admin không persist vào SQLite | Hash của Admin Password được persist; mật khẩu và secret thì không |

Các phần còn lại của ADR 0051 vẫn hiệu lực: Admin API chỉ mount khi `api.enabled = true` và trả `404` khi tắt, từ chối request có nhiều hơn một header `Authorization`, chính sách CORS, preflight trước xác thực, tách hoàn toàn khỏi `auth.token` của Voice/OTA.

## Considered Options

- **Giữ `admin_token` song song cho script:** từ chối theo yêu cầu. Hai đường vào nghĩa là token tĩnh không hạn vẫn tồn tại và mật khẩu không phải cửa duy nhất.
- **Phiên lưu ở server với hạn 10 phút không hoạt động:** từ chối. Request nền của Admin Web (SSE, thông báo) làm phiên không bao giờ hết hạn nếu gia hạn theo mọi request, còn cách sửa bằng heartbeat thêm state và một route chỉ để giữ phiên. Yêu cầu 10 phút đã được bỏ.
- **Khoá ký sinh ngẫu nhiên và lưu trong SQLite:** từ chối. Database Snapshot sao nguyên file nên người giữ backup sẽ giả được token.
- **Khoá ký dẫn xuất từ `admin_token`:** không còn áp dụng khi `admin_token` bị bỏ.
- **Limiter theo IP:** từ chối. Sau reverse proxy server không có địa chỉ client đáng tin.
- **Cookie HttpOnly:** từ chối trong phạm vi này. Cần đổi chính sách CORS và thêm bảo vệ CSRF, trong khi mọi module API của Admin Web đã dùng header Bearer.

## Consequences

- Thay đổi phá vỡ tương thích: deployment đang chạy phải thêm `jwt_secret` và xoá `admin_token` trước khi khởi động bản mới; script đang gửi `admin_token` phải chuyển sang đăng nhập.
- Deployment mới không tự động hoá được qua Admin API cho đến khi mật khẩu mặc định được đổi.
- Trước lần đổi đầu tiên, ai tới được cổng server đều đăng nhập được bằng mật khẩu mặc định. Deployment lộ ra mạng nên được khởi tạo sau firewall.
- Kẻ dò mật khẩu có thể tạm khoá route đăng nhập của cả Admin lẫn script. Token đã cấp và lệnh reset không bị ảnh hưởng.
- Người đọc được `jwt_secret` tự ký được Admin Web Token; secret phải được giữ như credential toàn quyền.
- Tắt Password Login khiến Admin API không còn xác thực, gồm cả Resource Credential và export backup.
- Việc triển khai đi theo expand–contract: thêm Admin Web Token bên cạnh `admin_token`, chuyển test theo lô, rồi mới xoá `admin_token`.
