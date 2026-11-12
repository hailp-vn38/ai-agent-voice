# Đăng nhập Admin Web bằng mật khẩu và JWT

Status: ready-for-agent

Quyết định kiến trúc: [ADR 0141](../../docs/adr/0141-admin-password-login-and-signed-web-tokens.md) (Proposed). Ticket: [issues/](issues/), triển khai theo expand–contract.

Thay thế phần xác thực của [ADR 0051](../../docs/adr/0051-separate-authenticated-admin-api.md):

- ADR 0051 nói mọi request Admin API bắt buộc `admin_token`. Spec này **bỏ hẳn `admin_token`**: credential duy nhất của Admin API là Admin Web Token, và không cần credential khi Admin tắt đăng nhập bằng mật khẩu.
- ADR 0051 nói V1 không có limiter chống dò trong tiến trình. Spec này thêm limiter cho riêng route đăng nhập.
- ADR 0051 nói credential Admin không được lưu vào SQLite. Spec này lưu **hash** của Admin Password; mật khẩu và khoá ký không nằm trong SQLite.

Giữ nguyên [ADR 0058](../../docs/adr/0058-minimal-admin-mutation-audit.md): đăng nhập sai chỉ ra telemetry, không thành Admin Audit Event.

## Problem Statement

Muốn dùng Admin Web, Admin phải dán `admin_token` lấy từ `config.toml` của server vào một ô nhập trên trang. Token này là chuỗi ngẫu nhiên dài, không đổi được từ giao diện, không có hạn, và ai từng thấy nó thì giữ quyền mãi cho đến khi có người sửa file config và khởi động lại server. Bản build còn cho phép nhúng thẳng token vào bundle, khi đó ai tải được trang cũng đọc được token.

Admin không có cách nào đăng nhập bằng một mật khẩu tự đặt, không đổi được mật khẩu, và không đăng xuất được các trình duyệt khác.

## Solution

Lần đầu mở Admin Web, Admin thấy trang đăng nhập và vào bằng mật khẩu mặc định. Ngay sau đó Admin Web buộc đổi mật khẩu; đổi xong mới dùng được các trang khác.

Từ đó, Admin đăng nhập bằng mật khẩu của mình. Server trả một Admin Web Token có hạn; hết hạn thì đăng nhập lại. Trong mục System có trang Bảo mật để đổi mật khẩu, đăng xuất mọi trình duyệt, và tắt hoặc bật lại việc đăng nhập bằng mật khẩu.

`admin_token` bị bỏ khỏi config. Thay vào đó config có một chuỗi khoá bí mật dùng để ký Admin Web Token. Script và CLI muốn gọi Admin API cũng phải đăng nhập bằng Admin Password để lấy token.

## Thuật ngữ

Ba khái niệm mới, chưa có trong `CONTEXT.md`; cần thêm khi triển khai:

- **Admin Password**: mật khẩu duy nhất của deployment để dùng Admin API. Không phải Resource Credential, không gắn với một người dùng cụ thể.
- **Admin Web Token**: JWT có hạn do server cấp sau khi kiểm Admin Password, là credential duy nhất của Admin API. Không phải token System MCP.
- **Admin Signing Secret**: chuỗi khoá bí mật trong config dùng để ký và kiểm Admin Web Token. Không phải credential để gửi lên API.
- **Password Login**: công tắc cấp deployment quyết định Admin API có đòi credential từ Admin Web hay không.

`CONTEXT.md` đang tránh các từ "Admin User" và "Admin Session"; spec này không dùng chúng. Hệ thống vẫn chỉ có một Admin, không có tài khoản.

## User Stories

1. As an Admin, I want a login page when I open Admin Web without a valid Admin Web Token, so that I never have to find and paste `admin_token`.
2. As an Admin, I want to sign in with a known default password on a fresh deployment, so that I can get in without reading server files.
3. As an Admin, I want to be forced to set my own password right after signing in with the default one, so that the default never stays in use.
4. As an Admin, I want every other page and API call refused until I have replaced the default password, so that a half-configured deployment is not usable.
5. As an Admin, I want to stay signed in after changing my password, so that I do not have to log in twice in a row.
6. As an Admin, I want to stay signed in across page reloads and new tabs, so that I am not asked for the password constantly.
7. As an Admin, I want my sign-in to expire after a fixed period, so that a forgotten browser does not keep access forever.
8. As an Admin, I want to be sent to the login page as soon as my token is rejected, so that I am not left on a page full of failed requests.
9. As an Admin, I want to return to the page I was on after signing in again, so that expiry does not lose my place.
10. As an Admin, I want a clear message when the password is wrong, so that I know to retry.
11. As an Admin, I want a clear message when I have tried too many times, so that I know to wait rather than keep typing.
12. As an Admin, I want a Security page under System, so that all login settings are in one place.
13. As an Admin, I want to change my password by giving the current one and a new one, so that someone at my unlocked browser cannot change it silently.
14. As an Admin, I want a minimum length enforced on new passwords, so that I do not set a trivially guessable one.
15. As an Admin, I want the new password refused if it equals the default, so that the forced change cannot be bypassed.
16. As an Admin, I want changing my password to sign out every other browser, so that an old token stops working when I suspect a leak.
17. As an Admin, I want a "sign out everywhere" action, so that I can revoke all tokens without changing the password.
18. As an Admin, I want a "sign out" action for this browser, so that I can leave a shared machine clean.
19. As an Admin, I want to turn Password Login off, so that on a machine only I can reach I am not asked to log in.
20. As an Admin, I want turning Password Login off to require my current password, so that it cannot be disabled by someone passing by.
21. As an Admin, I want a warning before turning Password Login off that says anyone who can reach the server gets full access, so that I make the choice knowingly.
22. As an Admin, I want Admin Web to open straight into the app when Password Login is off, so that the login page does not appear for no reason.
23. As an Admin, I want to turn Password Login back on from the Security page, so that I can restore protection without touching the server.
24. As an Admin, I want my previous password to still work after turning Password Login back on, so that re-enabling does not lock me out.
25. As an Admin, I want a server command that resets the Admin Password to the default, so that I can recover when I forget it.
26. As an Admin, I want a reset to force a password change again and re-enable Password Login, so that recovery returns to the same safe first-run state.
27. As an Admin, I want password changes, sign-out-everywhere and Password Login toggles recorded as Admin Audit Events, so that I can see when login settings changed.
28. As an Admin, I want failed logins visible in the System Log, so that I can notice someone guessing.
29. As an Admin, I want live pages (jobs, logs, notifications, wiki chat) to keep working after login, so that streaming features are not broken by the new login.
30. As an Admin, I want the Studio Assistant to keep working after login, so that the new login does not remove a feature.
31. As an operator, I want scripts and CLI calls to obtain an Admin Web Token by logging in with the Admin Password, so that automation uses the same single credential path as the browser.
32. As an operator, I want the server to refuse to start when the Admin API is enabled without a sufficiently long Admin Signing Secret, so that tokens are never signed with a weak or empty key.
33. As an operator, I want rotating the Admin Signing Secret to invalidate every Admin Web Token, so that one config change cuts off all browsers and scripts.
40. As an operator, I want the production init script to generate a random Admin Signing Secret, so that I never have to invent one.
41. As an operator, I want a clear startup error naming the removed `admin_token` setting when my old config still has it, so that I know what to change after upgrading.
34. As an operator, I want the token lifetime configurable, so that I can shorten it on exposed deployments.
35. As an operator, I want the Admin Web bundle to contain no credential, so that serving the static files leaks nothing.
36. As an operator, I want a Database Snapshot to be useless for forging Admin Web Tokens, so that a leaked backup does not grant access.
37. As an operator, I want importing a backup to leave the Admin Password of the target server untouched, so that a restore never changes who can log in.
38. As an operator, I want the Admin Password never written to logs, audit, or error bodies, so that it does not leak through diagnostics.
39. As a developer, I want one test helper that logs in and returns a usable Admin Web Token, so that the existing Admin API tests move off `admin_token` with a mechanical change.

## Implementation Decisions

### Mô hình

- Vẫn chỉ có một Admin. Không có tài khoản, tên đăng nhập, vai trò hay bảng người dùng.
- Trạng thái đăng nhập là một bản ghi singleton trong SQLite gồm: Password Login bật hay tắt, hash và salt của Admin Password, cờ "phải đổi mật khẩu", revision và thời điểm cập nhật. Thêm bằng một migration mới, forward-only.
- Khi bản ghi chưa có hash (deployment mới hoặc vừa nâng cấp), server seed Admin Password mặc định là `admin`, bật Password Login và bật cờ phải đổi mật khẩu.
- Hash dùng PBKDF2-HMAC-SHA256 có salt ngẫu nhiên theo từng lần đặt, bằng thư viện mật mã đã có trong workspace. Không thêm dependency.
- So khớp mật khẩu và token dùng so sánh hằng thời gian.

### Admin Web Token

- JWT ký HS256, claims gồm thời điểm cấp, thời điểm hết hạn và revision của trạng thái đăng nhập lúc cấp.
- Khoá ký là Admin Signing Secret, đọc từ config và **không lưu trong SQLite**. Hệ quả: Database Snapshot không chứa gì đủ để giả token; đổi secret làm mọi token cũ vô hiệu; restart server không làm mất đăng nhập.
- Khoá ký tách biệt với khoá của token System MCP. Bộ kiểm token System MCP không chấp nhận Admin Web Token và ngược lại.

### Config

- Bỏ `admin_token` khỏi mục `[api]`. Thêm `jwt_secret` (Admin Signing Secret) cùng mục, ghi đè được bằng biến môi trường theo cách `admin_token` đang được ghi đè.
- Khi `api.enabled = true`, `jwt_secret` phải dài ít nhất 32 ký tự sau khi trim; thiếu hoặc ngắn hơn thì server từ chối khởi động.
- Config còn khoá `admin_token` thì server từ chối khởi động với thông báo nói rõ khoá đã bị bỏ và phải thay bằng `jwt_secret`. Không tự chuyển giá trị cũ sang.
- `jwt_secret` không được trùng `auth.token` của Voice/OTA hay khoá JWT của System MCP. Các kiểm tra hiện đang so với `admin_token` (ví dụ ở bộ kiểm token System MCP) chuyển sang so với `jwt_secret`.
- `jwt_secret` không được log, không có trong `Debug`, không ghi vào SQLite.
- Script khởi tạo production sinh `jwt_secret` ngẫu nhiên ở chỗ đang sinh `admin_token`. Config mẫu, config Docker mẫu và config test của Docker đổi theo.
- Thời hạn mặc định 7 ngày, cấu hình được trong mục `[api]` của config, có giới hạn dưới và trên hợp lý. Không có refresh token; hết hạn thì đăng nhập lại.
- Token bị từ chối khi sai chữ ký, sai thuật toán, quá hạn, hoặc revision trong token khác revision hiện tại. Đổi mật khẩu, reset mật khẩu và "đăng xuất mọi nơi" đều tăng revision, nên thu hồi được toàn bộ token mà không cần lưu danh sách.
- Đăng xuất một trình duyệt chỉ xoá token ở trình duyệt đó; không có thu hồi từng token.

### Xác thực Admin API

Middleware xác thực hiện có của Admin API đổi thành chấp nhận, theo thứ tự:

1. Bearer là Admin Web Token hợp lệ.
2. Password Login đang tắt: cho qua kể cả khi không có header.
3. Còn lại: `401 unauthorized` như hiện nay.

- Không còn so khớp Bearer với một token tĩnh nào.
- Vẫn từ chối request có nhiều hơn một header `Authorization`.
- Khi cờ phải đổi mật khẩu đang bật, mọi request chỉ được gọi route đổi mật khẩu và route trạng thái; route khác trả `403 password_change_required`. Deployment mới vì thế không tự động hoá được cho đến khi có người (hoặc script) đổi mật khẩu mặc định.
- Revision hiện tại, cờ phải đổi mật khẩu và trạng thái Password Login được giữ trong bộ nhớ cùng `AppState` và cập nhật khi commit, để middleware không truy vấn SQLite mỗi request. Import backup và lệnh reset phải làm mới bản trong bộ nhớ.
- Ngoại lệ sẵn có cho OAuth callback của LLM Gateway giữ nguyên.

### Route mới

Tất cả nằm dưới Admin API, cùng tiền tố phiên bản với các route hiện có, nhóm `auth`:

| Route | Credential | Hành vi |
|---|---|---|
| đọc trạng thái | không cần | Trả Password Login bật/tắt và cờ phải đổi mật khẩu. Không trả gì khác. |
| đăng nhập | không cần | Nhận mật khẩu; đúng thì trả Admin Web Token, thời điểm hết hạn và cờ phải đổi mật khẩu. Sai: `401 invalid_password`. Quá giới hạn: `429 too_many_attempts` kèm `Retry-After`. Password Login tắt: `409 password_login_disabled`. |
| đổi mật khẩu | Admin Web Token | Nhận mật khẩu hiện tại và mật khẩu mới. Kiểm mật khẩu hiện tại, kiểm quy tắc, lưu hash mới, tắt cờ phải đổi, tăng revision, trả Admin Web Token mới. |
| đăng xuất mọi nơi | Admin Web Token | Tăng revision, trả Admin Web Token mới cho trình duyệt đang gọi. |
| đặt Password Login | Admin Web Token | Nhận trạng thái mong muốn và mật khẩu hiện tại. Khi Password Login đang tắt và muốn bật lại thì không cần credential nhưng vẫn cần mật khẩu hiện tại. |

- Quy tắc mật khẩu mới: tối thiểu 8 ký tự, có giới hạn trên để chặn body lớn, không được bằng mật khẩu mặc định, không được bằng mật khẩu hiện tại. Vi phạm: `422` với mã lỗi riêng cho từng quy tắc.
- Không cho tắt Password Login khi cờ phải đổi mật khẩu còn bật.
- Hai route không cần credential vẫn đi qua middleware request ID và Admin JSON Transport Boundary hiện có.
- Mã lỗi tuân theo khuôn lỗi JSON hiện có của Admin API.

### Chống dò mật khẩu

- Limiter trong tiến trình, chỉ áp cho route đăng nhập: tối đa 5 lần sai trong 60 giây tính chung toàn server, sau đó từ chối mọi lần thử cho đến hết cửa sổ. Đăng nhập đúng không xoá bộ đếm.
- Tính chung toàn server thay vì theo IP, vì sau reverse proxy server không có địa chỉ client đáng tin. Hệ quả chấp nhận được: kẻ dò có thể tạm khoá đăng nhập của cả Admin lẫn script, vì không còn đường vào nào khác; token đã cấp vẫn dùng được và lệnh reset không bị ảnh hưởng.
- Limiter nằm trong bộ nhớ, mất khi restart.

### Audit và log

- Đổi mật khẩu, đăng xuất mọi nơi, đặt Password Login và reset mật khẩu ghi Admin Audit Event trong cùng transaction với thay đổi, theo đúng khuôn metadata hiện có (resource, action, revision trước và sau, outcome). Không có trường nào chứa mật khẩu, hash hay token.
- Đăng nhập sai và bị limiter chặn chỉ phát Structured Log Entry mức cảnh báo, không thành Admin Audit Event (ADR 0058).
- Mật khẩu và token không xuất hiện trong log, trong `Debug`, hay trong body lỗi.

### Backup

- Database Backup Bundle không mang trạng thái đăng nhập; import không đọc và không ghi nó.
- Database Snapshot là bản sao nguyên file nên có chứa hash của Admin Password. Chấp nhận, vì hash có salt và khoá ký không nằm trong DB. Import từ snapshot vẫn không áp trạng thái đăng nhập vào server đích.

### Lệnh reset

- Thêm một lệnh con của binary server để đưa Admin Password về mặc định, bật Password Login, bật cờ phải đổi mật khẩu và tăng revision. Lệnh ghi thẳng SQLite; server đang chạy cần khởi động lại để nhận thay đổi, và lệnh in ra điều đó.

### Admin Web

- Bỏ ô dán token và biến môi trường nhúng token vào bundle, kể cả trong Dockerfile, file env mẫu và README.
- Store xác thực giữ Admin Web Token thay cho `admin_token`, lưu trong `localStorage` để dùng chung giữa các tab và sống qua reload. Hàm đọc token mà client HTTP và các luồng SSE đang gọi giữ nguyên chữ ký, nên các module API không phải sửa.
- Khi khởi động, Admin Web đọc trạng thái đăng nhập trước khi tải dữ liệu, rồi chọn một trong ba: vào thẳng app (Password Login tắt hoặc token còn dùng được), trang đăng nhập, hoặc trang đổi mật khẩu bắt buộc.
- Router có guard: route nào cũng đòi token trừ trang đăng nhập; khi cờ phải đổi mật khẩu bật thì chỉ vào được trang đổi mật khẩu. Sau khi đăng nhập, quay về route đã định đến.
- Client HTTP xử lý tập trung: `401` thì xoá token và về trang đăng nhập; `403 password_change_required` thì về trang đổi mật khẩu. Các luồng SSE tự mở bằng `fetch` cũng phải đi qua cùng xử lý này.
- Thêm trang Bảo mật trong System: đổi mật khẩu, đăng xuất, đăng xuất mọi nơi, công tắc Password Login kèm hộp xác nhận có cảnh báo và ô nhập mật khẩu hiện tại.
- Mọi chuỗi mới có đủ bản tiếng Anh và tiếng Việt trong bảng i18n hiện có.
- Studio Assistant: kiểm tra đường lấy quyền của nó khi triển khai; nếu nó dựa vào token đang lưu trong store thì phải tiếp tục chạy với Admin Web Token.

### Tài liệu

- ADR 0141 ghi quyết định và ba điểm thay ADR 0051.
- Thêm ba thuật ngữ vào `CONTEXT.md` và `GLOSSARY.md`.
- Cập nhật tài liệu Admin API, README của Admin Web, config mẫu và config mẫu của Docker.

## Testing Decisions

Test tốt ở đây chỉ kiểm hành vi nhìn từ ngoài: gửi request HTTP thật vào router và đọc status, mã lỗi, body; hoặc gắn component/store và đọc thứ Admin thấy. Không test hàm hash, hàm ký hay cấu trúc bảng trực tiếp.

### Seam 1: HTTP Admin API (đã có)

Dùng harness test Admin API hiện có: dựng `AppState` với SQLite tạm, gọi router qua HTTP client. Đây là seam chính và đủ cho toàn bộ phần server. Prior art: bộ test hợp đồng Admin API và bộ test credential/MCP trong cùng thư mục.

Cần phủ:

- Deployment mới: đăng nhập bằng mật khẩu mặc định thành công và báo phải đổi mật khẩu; token đó bị `403 password_change_required` ở một route bất kỳ; đổi mật khẩu xong thì token mới gọi được route đó.
- Mật khẩu sai trả `401`; quá 5 lần trả `429` có `Retry-After`.
- Token hết hạn bị `401`. Đặt thời hạn token rất ngắn qua config của test thay vì can thiệp đồng hồ.
- Token bị sửa một ký tự, token ký bằng khoá khác, token dùng thuật toán `none` đều bị `401`.
- Đổi mật khẩu và đăng xuất mọi nơi làm token cấp trước đó bị `401`, còn token vừa trả về dùng được.
- Quy tắc mật khẩu: quá ngắn, bằng mặc định, bằng mật khẩu hiện tại, sai mật khẩu hiện tại.
- Tắt Password Login: request không có header được chấp nhận; bật lại thì bị `401`, và mật khẩu cũ vẫn đăng nhập được. Không tắt được khi còn phải đổi mật khẩu.
- Giá trị `admin_token` cũ gửi làm Bearer bị `401`.
- Config thiếu `jwt_secret`, `jwt_secret` ngắn hơn 32 ký tự, hoặc còn khoá `admin_token` đều làm khởi động thất bại. Đặt cạnh các test validation config hiện có.
- Dựng lại app trên cùng SQLite và cùng config: token cũ còn dùng được. Dựng lại với `jwt_secret` khác: token cũ bị `401`.
- Mỗi thay đổi tạo đúng một Admin Audit Event không chứa mật khẩu; đăng nhập sai không tạo Admin Audit Event.
- Token System MCP không được Admin API chấp nhận, và Admin Web Token không được endpoint System MCP chấp nhận.
- Export rồi import Database Backup Bundle không đổi Admin Password của server đích. Đặt cạnh các test backup hiện có.

Mọi test đang gửi `admin_token` (Admin API, background HTTP, memory admin, web assistant, transcript archive, wiki chat và các test config) phải chuyển sang Admin Web Token. Harness có một helper duy nhất: đăng nhập bằng mật khẩu mặc định, đổi mật khẩu, trả token; các test chỉ thay chỗ lấy Bearer. Ngoài chỗ đó, phần khẳng định của các test hiện có không đổi.

### Seam 2: Admin Web (đã có)

Vitest với jsdom, `fetch` giả, như các test client HTTP, store và trang System hiện có.

- Client HTTP: gắn Admin Web Token vào header; `401` xoá token và điều hướng về đăng nhập; `403 password_change_required` điều hướng về đổi mật khẩu.
- Guard của router: ba nhánh khởi động (Password Login tắt, chưa có token, phải đổi mật khẩu) và quay lại route đã định sau đăng nhập.
- Trang đăng nhập và trang đổi mật khẩu: hiển thị đúng thông báo cho từng mã lỗi.
- Trang Bảo mật: tắt Password Login đòi xác nhận và mật khẩu hiện tại.

Không thêm test e2e Playwright; hai seam trên đã phủ hành vi.

### Cách chạy

Chỉ chạy qua `scripts/agent-test.sh`: `targeted` cho test server mới trước, rồi `package`; `web test` và `web typecheck` cho Admin Web.

## Out of Scope

- Nhiều tài khoản Admin, tên đăng nhập, vai trò, phân quyền theo trang.
- Tự hết hạn theo thời gian không hoạt động (yêu cầu 10 phút ban đầu đã được bỏ).
- Refresh token, thu hồi từng token, danh sách thiết bị đang đăng nhập.
- Cookie HttpOnly và bảo vệ CSRF; Admin Web tiếp tục dùng header Bearer theo chính sách CORS hiện tại.
- 2FA, SSO, OAuth cho Admin.
- Token tĩnh dài hạn hoặc API key riêng cho script; script dùng chung đường đăng nhập bằng mật khẩu.
- Xoay vòng Admin Signing Secret từ giao diện.
- Limiter theo IP hoặc khoá tài khoản lâu dài.
- Thay đổi xác thực của Voice/OTA (`auth.token`), Knowledge MCP hay System MCP.
- Giao diện đọc Admin Audit Event cho các sự kiện mới ngoài những gì trang System đã hiển thị.

## Further Notes

- Mật khẩu mặc định cố định là rủi ro có chủ đích: trước lần đổi đầu tiên, ai tới được cổng server đều đăng nhập được. Cờ phải đổi mật khẩu thu hẹp cửa sổ đó nhưng không đóng nó; deployment lộ ra mạng nên được khởi tạo sau firewall rồi mới mở.
- Tắt Password Login khiến Admin API không còn xác thực, gồm cả Resource Credential và export backup. Spec cho phép ở mọi kiểu bind theo yêu cầu, chỉ cảnh báo trên giao diện.
- Người đọc được `jwt_secret` tự ký được Admin Web Token mà không cần mật khẩu (vẫn phải biết revision hiện tại, vốn là số nhỏ dễ dò). Secret vì thế phải được giữ như một credential toàn quyền.
- Đây là thay đổi phá vỡ tương thích: deployment đang chạy phải thêm `jwt_secret` và xoá `admin_token` trước khi khởi động bản mới, mọi script đang gửi `admin_token` phải chuyển sang đăng nhập, và Admin sẽ thấy trang đăng nhập với mật khẩu mặc định ở lần mở Admin Web kế tiếp. Cần ghi rõ trong ghi chú phát hành.
