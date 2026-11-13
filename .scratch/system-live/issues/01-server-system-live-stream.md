# 01: Server phát System Live Snapshot qua `GET /api/admin/system/live`

**What to build:** Module `services::system::live` và route SSE `/api/admin/system/live`. Sau ticket này, một client đã xác thực mở stream thì nhận ngay snapshot hiện tại, rồi một snapshot mới mỗi khi System Resources hoặc System Status đổi. System Status được tổng hợp nhiều nhất một lần mỗi 15 giây, và chỉ khi có subscriber.

**Blocked by:** Không.

Status: ready-for-agent

Spec: [system-live](../spec.md) · [ADR 0146](../../../docs/adr/0146-system-live-snapshot-stream.md)

- [ ] `SystemMetrics` phát mỗi mẫu qua một `watch`; `latest()` giữ nguyên hành vi và `MAX_AGE`.
- [ ] `SystemLive::start(stop)` và `SystemLive::subscribe()` theo interface trong spec; được dựng trong `AppState` và bootstrap khởi động task giống `SystemMetrics::start`.
- [ ] Nguồn System Status là seam nội bộ: adapter thật gọi `SystemStatus::summary()`, adapter test là fake có đếm số lần gọi.
- [ ] Chỉ tổng hợp khi `receiver_count() > 0`. Subscriber đầu tiên sau lúc nhàn rỗi làm tổng hợp chạy ngay; không có hai lần tổng hợp chồng nhau.
- [ ] Snapshot đúng hợp đồng `v: 1` trong spec: `resources` là `null` khi `latest()` trả `None`; `status.summary` là `null` khi tổng hợp lỗi, còn `checked_at` vẫn có giá trị.
- [ ] `summary()` chỉ kiểm tra database một lần mỗi lần gọi.
- [ ] Route `/api/admin/system/live`: event `snapshot` đầu tiên ngay khi mở, keepalive 15 giây, `Cache-Control: no-store`, kết thúc khi `lifecycle.stopping()` bị huỷ.
- [ ] `/health`, `/ready`, `/api/admin/system`, `/api/admin/system/resources` không đổi hợp đồng (test hiện có vẫn qua).
- [ ] Test `SystemLive` với tokio time paused theo danh sách trong spec (không subscriber → 0 lần gọi; ba subscriber trong 45 giây → 3–4 lần; nguồn lỗi; stop token).
- [ ] Test HTTP: event đầu là `snapshot` với `v: 1`; thiếu token → 401; shutdown → stream kết thúc.
- [ ] Ghi tài liệu route vào `docs/api/` cạnh tài liệu Admin API system hiện có.
- [ ] Chạy test qua `scripts/agent-test.sh`, test đích trước rồi mới tới toàn workspace.

## Comments
