# ADR 0146 — System Live Snapshot đẩy qua SSE, và stream Admin kết thúc theo phiên

Status: Accepted for implementation, 2026-11-13.
Extends: [ADR 0141](0141-admin-password-login-and-signed-web-tokens.md) (stream đang mở cũng chịu thu hồi phiên).
Related: [ADR 0100](0100-job-change-signals-and-authoritative-reads.md).

Admin Web đọc System Resources và System Status bằng polling theo từng trang: 4–16 request/phút mỗi tab. Mỗi lần đọc System Status chạy truy vấn SQLite, và mỗi trang tự xử lý tab ẩn, lỗi và dữ liệu cũ theo cách riêng. Spec: [system-live](../../.scratch/system-live/spec.md).

## Decisions

- **Server giữ một System Live Snapshot trong bộ nhớ.** Snapshot ghép mẫu System Resources mới nhất với System Status gần nhất; mỗi phần mang thời điểm của nó và có thể là `null`.
- **System Status được tổng hợp theo nhịp riêng (15 giây), chỉ khi có subscriber**, nhiều nhất một lần mỗi chu kỳ dù có bao nhiêu kết nối. Subscriber đầu tiên sau lúc nhàn rỗi làm tổng hợp chạy ngay. Không ai xem thì không truy vấn database cho phần này.
- **Phát bằng `tokio::sync::watch`.** Subscriber chậm chỉ đọc giá trị mới nhất. Không hàng đợi, không `Lagged`, không có bộ đệm phải giới hạn.
- **Stream mang toàn bộ snapshot (`GET /api/admin/system/live`, event `snapshot`, `v: 1`)**, không phải tín hiệu thay đổi. Event đầu tiên là giá trị hiện tại, nên nối lại không cần đọc REST.
- **Kết nối còn mở không có nghĩa là Ready.** Readiness nằm trong snapshot (`status.summary.status`) cùng `checked_at`. Keepalive chỉ chứng minh vận chuyển còn sống. `/health` và `/ready` vẫn dành cho hạ tầng.
- **Hợp đồng REST giữ nguyên.** `/api/admin/system` và `/api/admin/system/resources` vẫn là các lần đọc độc lập cho script và System MCP.
- **Mọi stream SSE của Admin API kết thúc khi điều kiện xác thực của middleware không còn đúng.** Cứ 5 giây và tại `exp` của token, server kiểm tra lại: Password Login đang tắt, hoặc bearer vẫn được `AdminAuthState::accepts` chấp nhận. Kiểm tra chỉ dùng bộ nhớ. Không có event kết thúc riêng: client nối lại, nhận 401, và luồng phiên hiện có xử lý tiếp. Quy tắc áp cho stream mới và các stream jobs, notifications, system logs, knowledge events.
- **Phía web có một module duy nhất, `useSystemLive()`.** Module chia sẻ một kết nối mỗi tab, đóng kết nối khi tab ẩn, đóng sau 10 giây khi không còn caller, nối lại với backoff 1–30 giây, giữ lịch sử CPU tối đa 60 mẫu, và tính độ tươi theo đồng hồ trình duyệt lúc nhận dữ liệu. Mất mạng thì giữ dữ liệu cuối cùng và đánh dấu cũ; hết phiên thì xoá.

## Vì sao khác ADR 0100

ADR 0100 chọn tín hiệu thay đổi rồi đọc lại REST, vì Job là trạng thái bền trong SQLite: REST là nguồn chân lý, còn tín hiệu có thể mất. System Live Snapshot là quan sát tạm thời, không có bản bền nào để đọc lại. Nó đổi mỗi 5 giây, nên "báo đổi rồi đọc lại" vẫn sinh một GET mỗi 5 giây. Snapshot nhỏ và giá trị mới nhất thay giá trị cũ, nên gửi thẳng snapshot là chính lần đọc. Hai quyết định không mâu thuẫn: tiêu chí chung là "đọc từ nguồn chân lý", và ở đây nguồn chân lý chính là bộ nhớ của tiến trình.

## Considered Options

- **Giữ polling theo trang, chỉ sửa tab ẩn ở Overview:** rẻ nhất. Nhưng vẫn mỗi tab một truy vấn SQLite mỗi chu kỳ, logic độ tươi và lịch sử CPU vẫn lặp ở từng trang.
- **SSE báo thay đổi + REST (mẫu ADR 0100):** từ chối vì lý do ở trên.
- **Module polling dùng chung + cache System Status phía server:** giải quyết được tải SQLite và trùng lặp phía web, nhưng vẫn có request định kỳ, và độ trễ tối thiểu bằng chu kỳ poll. Interface `useSystemLive()` cho phép quay về cách này mà không sửa trang, nếu SSE gặp vấn đề ở proxy.
- **WebSocket:** từ chối. Luồng quan sát chỉ một chiều. Admin API đã có hạ tầng SSE (parser, xác thực Bearer, test), còn WebSocket phải thêm giao thức và xác thực riêng.
- **Chia sẻ kết nối giữa các tab (BroadcastChannel/SharedWorker):** hoãn. Đóng kết nối khi tab ẩn đã giữ được ngân sách kết nối trong cách dùng thường gặp.
- **Event `session_ended` trước khi đóng stream:** từ chối. Lần nối lại nhận 401 đã đủ để luồng phiên hiện có xử lý, và tránh thêm một event mọi client phải hiểu.

## Consequences

- Mỗi tab đang hiển thị trang System giữ thêm một kết nối HTTP. Trên HTTP/1.1 (nginx của compose nghe plain HTTP), trình duyệt giới hạn 6 kết nối mỗi origin, và notifications stream đã chiếm một kết nối mỗi tab. Nhiều tab cùng hiển thị có thể chạm giới hạn. Đóng stream khi tab ẩn là biện pháp chính; HTTP/2 ở proxy TLS phía trước giải quyết triệt để.
- Server luôn có một task phát snapshot. Khi không ai xem, task chỉ chuyển tiếp mẫu resources trong bộ nhớ.
- Số liệu System Status trên giao diện có thể trễ tới 15 giây (cùng mức với polling hiện tại ở Overview).
- Stream Admin đang mở dừng tối đa 5 giây sau khi đăng xuất mọi nơi hoặc đổi mật khẩu. Đây là thay đổi hành vi với các stream hiện có; trước đây chúng chạy tới khi mất kết nối.
- `SystemMetrics` có thêm một `watch` phát mỗi mẫu. System MCP và REST không bị ảnh hưởng.
