# System Live Snapshot: đẩy System Resources và System Status qua SSE

Status: ready-for-human

Quyết định kiến trúc: [ADR 0146](../../docs/adr/0146-system-live-snapshot-stream.md). Từ vựng: [GLOSSARY](../../GLOSSARY.md#system-observability--structured-logs).

## Problem Statement

Admin Web xem tình trạng server bằng cách hỏi lại server theo chu kỳ cố định. Mỗi trang tự đặt hẹn giờ, tự giữ trạng thái và tự xử lý lỗi:

| Trang | Request định kỳ | Dừng khi tab ẩn |
|---|---|---|
| Overview | `/resources` 5s; `/health` + `/ready` + `/api/admin/system` 15s → ~16 req/phút | Không |
| System | `/resources` 5s; status 30s → ~14 req/phút | Có |
| Monitor | `/resources` 5s → 12 req/phút | Có |
| Runtime | status 15s → 4 req/phút | Có |

Hệ quả:

- Mỗi lần đọc `/api/admin/system` chạy `provider_overview` trên SQLite và kiểm tra database hai lần (một lần trực tiếp, một lần trong `readiness()`). Chi phí nhân theo số tab đang mở.
- Overview vẫn hỏi server khi tab ẩn.
- Lịch sử CPU bị tính lại ở từng trang (Overview, Monitor) và mất khi chuyển trang.
- Mỗi trang hiểu "dữ liệu cũ" theo cách riêng. Store hiện tại xoá status ngay khi một request lỗi, nên mất mạng ngắn cũng làm giao diện trống.
- Các stream Admin đang có (jobs, notifications, system logs, knowledge) chỉ kiểm tra Admin Web Token lúc mở. Token hết hạn hoặc "đăng xuất mọi nơi" không dừng một stream đang chạy (mâu thuẫn với tinh thần thu hồi bằng revision của [ADR 0141](../../docs/adr/0141-admin-password-login-and-signed-web-tokens.md)).

## Solution

Server giữ **một** System Live Snapshot trong bộ nhớ, ghép mẫu System Resources mới nhất với System Status tổng hợp theo nhịp riêng. Snapshot được phát cho mọi trình duyệt qua một System Live Stream (SSE). System Status chỉ được tổng hợp khi có người đang xem, và nhiều nhất một lần mỗi chu kỳ, dù có bao nhiêu tab.

Admin Web đọc snapshot qua một module duy nhất, `useSystemLive()`. Module này chia sẻ một kết nối cho mọi trang trong cùng tab, đóng kết nối khi tab ẩn, tự nối lại, giữ lịch sử CPU có giới hạn, và báo độ tươi của từng phần.

Mọi stream SSE của Admin API kết thúc khi Admin Web Token mang theo không còn được chấp nhận.

## User Stories

1. As an Admin, I want số liệu CPU/RAM/disk tự cập nhật khoảng mỗi 5 giây, so that tôi thấy tải hiện tại mà không cần bấm làm mới.
2. As an Admin, I want trạng thái readiness, provider và số Voice Session đang hoạt động tự cập nhật, so that tôi biết server có đang nhận việc không.
3. As an Admin, I want thấy rõ phần nào đang hiển thị dữ liệu cũ và từ lúc nào, so that tôi không nhầm số liệu cũ là số liệu hiện tại.
4. As an Admin, I want giao diện giữ số liệu cuối cùng (đánh dấu cũ) khi mất mạng ngắn, so that màn hình không trống trong lúc kết nối lại.
5. As an Admin, I want biểu đồ CPU giữ lịch sử khi tôi chuyển giữa Overview và Monitor, so that tôi không phải đợi biểu đồ đầy lại.
6. As an Admin, I want tab ẩn không giữ kết nối hay gửi request, so that nhiều tab mở không làm nặng server và trình duyệt.
7. As an Admin, I want quay lại tab thì thấy ngay số liệu mới nhất, so that tôi không phải đợi chu kỳ kế tiếp.
8. As an Admin, I want bị đưa về trang đăng nhập khi phiên hết hạn trong lúc đang xem trang System, so that phiên hết hạn có hiệu lực cả với màn hình đang mở.
9. As an Admin, I want "đăng xuất mọi nơi" dừng mọi stream đang mở trên các trình duyệt khác, so that thu hồi phiên có hiệu lực ngay.
10. As an Admin, I want trang Transport vẫn kiểm tra trực tiếp `/health` và `/ready`, so that tôi kiểm được đúng endpoint mà hạ tầng dùng.
11. As a người vận hành, I want `/health`, `/ready`, `/api/admin/system` và `/api/admin/system/resources` giữ nguyên hợp đồng, so that script giám sát hiện có không phải sửa.
12. As a người vận hành, I want server không truy vấn database cho phần System Status khi không ai xem, so that deployment nhàn rỗi không tốn tài nguyên.
13. As a người vận hành, I want số lần tổng hợp System Status không tăng theo số tab đang mở, so that nhiều Admin xem cùng lúc không làm chậm SQLite.
14. As a người vận hành, I want stream kết thúc gọn khi server tắt, so that graceful shutdown không bị giữ lại bởi trình duyệt.
15. As a nhà phát triển web, I want một composable nhỏ trả snapshot, trạng thái kết nối, độ tươi và lịch sử CPU, so that trang mới chỉ việc đọc dữ liệu.
16. As a nhà phát triển web, I want test được module bằng transport giả, so that test hành vi reconnect, ẩn tab và hết phiên không cần server thật.
17. As a nhà phát triển server, I want test được việc tổng hợp System Status bằng nguồn giả có đếm số lần gọi, so that khẳng định được giới hạn "một lần mỗi chu kỳ".
18. As a nhà phát triển server, I want một chỗ duy nhất áp quy tắc "stream kết thúc khi token không còn được chấp nhận", so that stream SSE mới không quên quy tắc này.

## Implementation Decisions

### Từ vựng

- **System Resources**: giữ nguyên định nghĩa trong CONTEXT.md.
- **System Status**: bản tóm tắt an toàn do `GET /api/admin/system` trả (readiness, version, uptime, database, providers, sessions, provider runtime). Không dùng "Runtime Status", vì thuật ngữ đó đã chỉ trạng thái của một Loaded Runtime.
- **System Live Snapshot** và **System Live Stream**: định nghĩa mới trong GLOSSARY.

### Module server `services::system::live` (mới)

Một deep module sở hữu toàn bộ việc tạo snapshot. Interface:

```rust
impl SystemLive {
    /// Bootstrap sở hữu task này, giống SystemMetrics::start.
    pub fn start(self: Arc<Self>, stop: CancellationToken) -> JoinHandle<()>;
    /// Receiver luôn có giá trị ngay; giá trị là Arc<SystemLiveSnapshot>.
    pub fn subscribe(&self) -> watch::Receiver<Arc<SystemLiveSnapshot>>;
}
```

Bên trong module (không lộ qua interface):

- Nhận mẫu System Resources mới từ `SystemMetrics`. `SystemMetrics` được bổ sung một `watch` phát mỗi mẫu. `latest()` giữ nguyên hành vi, gồm cả `MAX_AGE` 15 giây.
- Tổng hợp System Status mỗi `STATUS_INTERVAL` = 15 giây, **chỉ khi `receiver_count() > 0`**. Subscriber đầu tiên sau một khoảng không ai xem làm tổng hợp chạy ngay, không đợi tick. Không bao giờ có hai lần tổng hợp chạy chồng nhau.
- Nguồn System Status là seam nội bộ có hai adapter: adapter thật gọi `SystemStatus::summary()`, adapter test là fake có đếm số lần gọi.
- Phát snapshot mới mỗi khi một phần thay đổi. Kênh là `watch`, nên subscriber chậm luôn đọc giá trị mới nhất, không có hàng đợi và không có `Lagged`.
- `summary()` chỉ kiểm tra database một lần mỗi lần gọi (bỏ lần gọi `is_reachable()` lặp trong `readiness()`).

### Hợp đồng System Live Stream

`GET /api/admin/system/live`, `Accept: text/event-stream`, đi sau middleware Admin như các route khác. Có `Cache-Control: no-store`, keepalive comment mỗi 15 giây.

- Ngay khi mở: một event `snapshot` với giá trị hiện tại. Nếu System Status chưa có thì `status.summary` là `null`, kèm `checked_at: null`.
- Sau đó: một event `snapshot` mỗi khi snapshot đổi. Mỗi event mang **toàn bộ** snapshot (khoảng 1–2 KB), không phải diff.

```json
{
  "v": 1,
  "resources": { /* ResourceSnapshot như GET /resources */ } ,
  "status": {
    "checked_at": "2026-11-13T08:00:00.000Z",
    "refresh_interval_ms": 15000,
    "summary": { /* JSON như GET /api/admin/system */ }
  }
}
```

- `resources` là `null` khi `SystemMetrics::latest()` trả `None` (sampler chưa có mẫu, ngừng chạy, hoặc mẫu cũ hơn 15s).
- `status.summary` là `null` khi tổng hợp thất bại (database không có hoặc lỗi SQL). `checked_at` vẫn là thời điểm thử. Không có mã lỗi nội bộ hay đường dẫn, giữ tinh thần "safe aggregate".
- Readiness nằm trong `status.summary.status`, đúng giá trị `/ready` dùng. Kết nối còn mở **không** có nghĩa là Ready.
- Stream kết thúc (không có event kết thúc) khi server dừng hoặc khi token không còn được chấp nhận.

### Quy tắc chung cho stream Admin (mở rộng ADR 0141)

- Một helper duy nhất trong `app::admin` bọc stream SSE của route. Helper giữ bearer của request và cứ 5 giây kiểm tra lại **đúng điều kiện mà middleware Admin dùng để cho request qua**: Password Login đang tắt, hoặc `AdminAuthState::accepts(bearer)` đúng. Điều kiện sai thì stream kết thúc. Kiểm tra này chỉ là phép so sánh trong bộ nhớ, không đọc SQLite.
- Khi token có `exp`, helper còn đặt hẹn giờ tại `exp` để kết thúc đúng lúc, không trễ thêm 5 giây.
- Bật Password Login không tăng revision, nên hai trường hợp này được xử lý bằng việc kiểm tra lại điều kiện, không dựa vào revision: stream mở lúc Password Login tắt (không có bearer) kết thúc khi Password Login được bật lại; stream có token hợp lệ vẫn chạy tiếp khi Password Login bị tắt.
- Áp dụng cho mọi route SSE Admin: `/jobs/stream`, `/notifications/stream`, `/system/logs/stream`, `/knowledge/events`, `/system/live`. Không áp dụng cho các response streaming theo yêu cầu (`/knowledge/ask`), vì chúng tự kết thúc theo câu trả lời.
- Client không cần event riêng. Stream kết thúc thì client nối lại; lần nối lại nhận 401 và luồng phiên hiện có (`handleSessionResponse`) đưa Admin về đăng nhập.

### Module web `useSystemLive()` (mới)

```ts
type Freshness = 'fresh' | 'stale' | 'unavailable'
type LiveConnection = 'connecting' | 'live' | 'reconnecting' | 'paused'

function useSystemLive(): {
  snapshot: Readonly<Ref<SystemLiveSnapshot | null>>
  cpuHistory: Readonly<Ref<readonly number[]>>   // tối đa 60 mẫu (~5 phút)
  connection: Readonly<Ref<LiveConnection>>
  freshness: Readonly<Ref<{ resources: Freshness; status: Freshness }>>
}
```

Interface gồm cả các quy ước sau:

- Phải gọi trong `setup` hoặc một effect scope; module tự gỡ đăng ký bằng `onScopeDispose`.
- **Một kết nối mỗi tab** dù có bao nhiêu caller (đếm tham chiếu ở cấp module).
- Caller cuối cùng rời đi thì kết nối đóng sau 10 giây, để chuyển giữa các trang System không phải mở lại.
- Tab ẩn: đóng kết nối ngay, `connection = 'paused'`, giữ snapshot. Tab hiện lại: mở ngay, snapshot đầu tiên thay giá trị cũ.
- Lỗi mạng hoặc stream kết thúc: `reconnecting`, backoff 1s → 30s (nhân đôi), về 1s sau khi nhận được snapshot.
- 401/403: gọi `handleSessionResponse`, xoá snapshot và lịch sử CPU, ngừng nối lại cho đến khi có caller mới sau khi đăng nhập.
- Độ tươi tính theo đồng hồ của trình duyệt lúc **nhận** dữ liệu, không theo timestamp của server, để không bị lệch đồng hồ:
  - `unavailable`: phần đó là `null` trong snapshot mới nhất, hoặc chưa có snapshot nào.
  - `stale`: lần cuối `sampled_at`/`checked_at` đổi đã quá 3 × chu kỳ (`sample_interval_ms` / `refresh_interval_ms`), hoặc đang không `live`.
  - `fresh`: còn lại.
- Lịch sử CPU chỉ nhận mẫu có `sampled_at` mới, nên nối lại không tạo điểm trùng.
- Seam nội bộ là transport: `(onSnapshot, signal) => Promise<'ended' | 'unauthorized'>`, có hai adapter: fetch-SSE thật (dùng lại `consumeSse` / `createSseParserState` trong `api/sse.ts`) và transport giả trong test. Transport không lộ qua `useSystemLive()`; test thay nó bằng một hàm cài đặt chỉ dành cho test.

### Các trang

- Overview, System, Monitor, Runtime chuyển sang `useSystemLive()`, bỏ mọi `setInterval` lấy resources/status và bỏ lịch sử CPU riêng. Đồng hồ hiển thị ("x giây trước") vẫn là việc của trang.
- Trang Transport giữ nguyên các lần kiểm tra trực tiếp `/health` và `/ready`.
- Store `server` chỉ còn phục vụ trang Transport (probe `/health` và `/ready`). Nếu không còn nơi nào khác dùng `status`, bỏ `status` khỏi store.
- Nút "làm mới" trên các trang dùng snapshot không gọi REST nữa. Nó buộc nối lại ngay nếu đang `reconnecting`; nếu đang `live` thì bị ẩn.

## Testing Decisions

Test đi qua interface của module, không đi xuyên vào bên trong.

- **Server, `SystemLive`** (tokio time paused, nguồn System Status giả có đếm):
  - không có subscriber → nguồn không bị gọi;
  - subscriber đầu tiên → nguồn bị gọi ngay một lần;
  - ba subscriber trong 45 giây → nguồn bị gọi đúng 3–4 lần, không phải 9–12;
  - subscriber rời hết → không gọi thêm;
  - mẫu resources mới → snapshot mới; `latest()` trả `None` → `resources: null`;
  - nguồn lỗi → `summary: null`, `checked_at` có giá trị;
  - stop token → task kết thúc.
- **Server, HTTP** (theo mẫu test SSE hiện có của `/system/logs/stream`):
  - event đầu tiên là `snapshot` với `v: 1`;
  - thiếu token → 401;
  - token hết hạn trong lúc stream mở → stream kết thúc;
  - tăng revision (đăng xuất mọi nơi) → stream kết thúc trong 5 giây;
  - shutdown → stream kết thúc.
  Helper token được test một lần qua một route; mỗi route còn lại có một test khói "kết thúc khi revision đổi".
- **Web, `useSystemLive()`** (Vitest, fake timers, transport giả):
  - hai component dùng cùng lúc → transport mở một lần;
  - caller cuối rời → đóng sau 10s; quay lại trong 10s → không mở lại;
  - `visibilitychange` ẩn → transport bị abort, `paused`; hiện lại → mở ngay;
  - transport ném lỗi → backoff 1s, 2s, 4s… tối đa 30s; snapshot → reset về 1s;
  - `unauthorized` → `handleSessionResponse` được gọi, snapshot về `null`, không nối lại;
  - chuyển từ `fresh` sang `stale` sau 3 × chu kỳ không có mẫu mới;
  - lịch sử CPU tối đa 60 mẫu, không trùng `sampled_at`.
- **Trang**: test component hiện có của Overview, Monitor, System, Runtime chuyển sang dùng `useSystemLive` giả; xoá các test về timer polling cũ.
- Mọi lệnh test chạy qua `scripts/agent-test.sh`.

## Out of Scope

- Đổi hợp đồng `/health`, `/ready`, `/api/admin/system`, `/api/admin/system/resources`.
- Cache kết quả REST `/api/admin/system` bằng snapshot (có thể làm sau nếu script giám sát tạo tải).
- Lịch sử resources phía server hoặc lưu trữ lâu dài.
- WebSocket; chia sẻ kết nối giữa nhiều tab (BroadcastChannel / SharedWorker).
- HTTP/2 tại nginx của compose (xem rủi ro trong ADR 0146).
- System MCP: vẫn đọc trực tiếp `SystemMetrics` / `SystemStatus`.

## Further Notes

- Ngân sách kết nối: trên HTTP/1.1 trình duyệt cho tối đa 6 kết nối mỗi origin. Mỗi tab Admin đang hiển thị giữ notifications stream và, khi ở trang System, thêm System Live Stream. Đóng System Live Stream khi tab ẩn giữ ngân sách này cho nhiều tab. Ticket 06 kiểm tra hành vi thực tế qua nginx.
- Thứ tự triển khai cho phép dừng giữa chừng: sau ticket 01 server đã chịu tải ít hơn khi có stream, nhưng trang vẫn polling cho tới ticket 04.
