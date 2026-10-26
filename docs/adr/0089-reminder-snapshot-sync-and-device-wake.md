# ADR 0089 — Firmware cache lịch có ACK bền vững và một wake timer

## Status

Accepted, 2026-10-26 — `server-ready, firmware-unverified` (Proposed, 2026-10-12; offline RTC wake cần nghiệm thu trên phần cứng thật).

Server giữ lịch authoritative; firmware chỉ cache kế hoạch wake đã xác nhận, không giữ quyền phát. Chọn snapshot có revision và một logical wake timer thay vì timer theo từng Reminder hay giả định server có thể đánh thức Device đang Deep Sleep.

- Sau mọi commit thay đổi pending, kể cả completed/expired, gửi `reminder.sync_required` tới current opted-in session; hint coalesce giữ revision cao nhất. Reconnect luôn `sync_request`, không phụ thuộc hint tới được.
- Snapshot nhất quán chứa tối đa 64 mục pending, sort `(due_at, id)`, revision, snapshot token, `has_more` và mốc sync tiếp. Firmware thay cache nguyên tử bằng bounded versioned NVS blob; chỉ `sync_ack` sau durable commit/verify. Two-slot journal chỉ thêm khi bằng chứng power-fail recovery yêu cầu.
- Server đối chiếu Device/current session/token/revision và đúng membership snapshot đã gửi; ACK không lùi revision. Item ngoài window là `out_of_window`, không suy `acked` chỉ từ revision toàn Device; replenish sau xử lý batch.
- Firmware sync/revalidate trước delivery, không tự phát nội dung cache. Wake sớm thì giữ WS tới due/delivery hoặc failure policy, không phát trước hạn; nhiều due dùng một socket và phát tuần tự. Sau Playback ACK phải đợi đúng `delivery_result`, ACK snapshot ít nhất terminal revision rồi mới ngủ; trước chủ động đóng WS cần current revision ACK hoặc backoff an toàn.
- Chỉ arm mốc tương lai; overdue/mạng lỗi dùng bounded reconnect backoff, tránh wake 0 ms. Giữ wake word thì chỉ đóng WS và dùng software timer; RTC Deep Sleep chỉ khi power profile/hardware cho phép.

Lịch mới tạo khi Device ngủ chưa tới firmware nên không bảo đảm đúng giờ; UI phải báo `sync_pending`. Feature `reminders` mặc định false, client cũ không nhận controls mới. Chỉ có server tests thì báo `server-ready, firmware-unverified`, không tuyên bố full offline E2E.

Nguồn: [Guide §4.4, §5.1–5.3, §7, §12, §16](../voice_reminders_direct_tts_implementation_guide.md#7-luồng-offline-esp32-tự-wake-và-reconnect).
