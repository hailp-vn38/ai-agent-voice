# 01: Server phát System Live Snapshot qua `GET /api/admin/system/live`

**What to build:** Module `services::system::live` và route SSE `/api/admin/system/live`. Sau ticket này, một client đã xác thực mở stream thì nhận ngay snapshot hiện tại, rồi một snapshot mới mỗi khi System Resources hoặc System Status đổi. System Status được tổng hợp nhiều nhất một lần mỗi 15 giây, và chỉ khi có subscriber.

**Blocked by:** Không.

Status: resolved

Spec: [system-live](../spec.md) · [ADR 0146](../../../docs/adr/0146-system-live-snapshot-stream.md)

- [x] `SystemMetrics` phát mỗi mẫu qua một `watch`; `latest()` giữ nguyên hành vi và `MAX_AGE`.
- [x] `SystemLive::start(stop)` và `SystemLive::subscribe()` theo interface trong spec; được dựng trong `AppState` và khởi chạy cùng lifecycle của server.
- [x] Nguồn System Status là seam nội bộ: adapter thật gọi `SystemStatus::summary()`, adapter test là fake có đếm số lần gọi.
- [x] Chỉ tổng hợp khi `receiver_count() > 0`. Subscriber đầu tiên sau lúc nhàn rỗi làm tổng hợp chạy ngay; tác vụ tuần tự không chạy chồng lấn.
- [x] Snapshot đúng hợp đồng `v: 1`: `resources` là `null` khi `latest()` trả `None`; `status.summary` là `null` khi tổng hợp lỗi, còn `checked_at` vẫn có giá trị.
- [x] `summary()` tái sử dụng kết quả database reachability nên chỉ kiểm tra database một lần mỗi lần gọi.
- [x] Route `/api/admin/system/live`: event `snapshot` đầu tiên ngay khi mở, keepalive 15 giây, `Cache-Control: no-store`, kết thúc khi `lifecycle.stopping()` bị huỷ.
- [x] `/health`, `/ready`, `/api/admin/system`, `/api/admin/system/resources` giữ nguyên hợp đồng.
- [x] Test `SystemLive` với tokio time paused: không subscriber, nhiều subscriber trong 45 giây, không gọi khi subscriber rời, cập nhật/xoá resources, nguồn lỗi và stop token.
- [x] Test HTTP: event đầu là `snapshot` với `v: 1`; thiếu token → 401; shutdown → stream kết thúc.
- [x] Ghi tài liệu route vào `docs/api/system.md`.
- [x] Chạy test đích qua `scripts/agent-test.sh`.

## Comments

- 2026-11-13: Implemented System Live Snapshot and SSE route. Targeted `scripts/agent-test.sh` gates passed: `check vela-server`, `sampler_publishes_and_stops_on_listening_cancellation`, `status_is_only_summarized_while_observed_and_is_shared_by_subscribers`, `failed_status_has_checked_at_but_no_summary_and_stop_ends_task`, `resource_samples_replace_the_snapshot_and_none_clears_resources`, and `system_live_requires_admin_auth_and_emits_initial_snapshot`.
