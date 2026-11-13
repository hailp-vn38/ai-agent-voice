# 04: `reminder_scheduler` phát Wake Signal cho Device không có phiên

**What to build:** Khi Reminder đến hạn mà Device không có Voice Session sẵn sàng, scheduler phát Wake Signal qua `device_push`. Device kết nối sau đó được phục vụ bằng đường Direct Speech và Playback ACK hiện có.

**Blocked by:** 01: OTA cấp thông tin Device Push Channel cho Device đã đăng ký.

Status: resolved

Spec: [device-push-wake](../spec.md) · [ADR 0145](../../../docs/adr/0145-device-push-channel-mqtt-wake-signal.md)

- [x] Seam publish nội bộ của `device_push` với hai adapter: client MQTT thật (tự nối lại, đăng nhập bằng tài khoản dịch vụ) và bản ghi nhận cho test.
- [x] Payload Wake Signal đúng quy ước trong spec (`v`, `type`, `id`, `at`, `expires_at`), QoS 1, retained.
- [x] Mỗi vòng scheduler: Device có Reminder đến hạn hoặc đến mốc thử lại mà không có phiên sẵn sàng nhận đúng một Wake Signal; có phiên sẵn sàng thì không phát.
- [x] Không phát lại cùng tín hiệu ở các vòng sau; phát lại khi tới mốc thử lại tiếp theo.
- [x] Deadline ngủ của scheduler tính cả Reminder của Device không có phiên khi tính năng bật.
- [x] Lỗi publish được ghi log có Device và Reminder, không dừng scheduler; log phân biệt publish thất bại với đã publish mà Device chưa kết nối.
- [x] Test tích hợp: Reminder đến hạn → Wake Signal được ghi nhận → Device giả lập mở phiên có reminders → Direct Speech → Playback ACK → Reminder hoàn tất.
- [x] Tính năng tắt: scheduler hành xử y như hiện tại; luật claim, lease, backoff, hết hạn không đổi.

## Comments

- Seam: `services::device_push_wake` (`WakePublisher`, `RecordingPublisher`) and `services::device_push_mqtt` (`MqttPublisher` on rumqttc, reconnects, QoS 1 retained). Scheduler entry: `reminder_scheduler::start_with_wake`; query `DeviceNotifications::wake_candidates`.
- Dedup is in memory per (reminder, trigger moment); a restart may re-publish once (retained, harmless).
- The MQTT client logs in with ticket 02's `DevicePush::service_username()` / `service_password()`.
- Tests: `tests/device_push_wake_scheduler.rs` simulates the Device session in-process (no real WebSocket, no real broker).

Review follow-up (2026-11-13):

- `device_push` is now one module (`services/device_push/`: `mod.rs`, `publisher.rs`, `mqtt.rs`). The scheduler calls `DevicePush::send_wake_signal`; topic, payload and the service login are private to the module. Paths above (`device_push_wake`, `device_push_mqtt`, `service_username()`) no longer exist.
- A publish counts as sent only after the broker's PUBACK (5 s timeout); a lost connection fails the waiting publish. One failed publish ends the pass so a dead broker does not hold up the loop.
- "wake signal sent but device did not connect" is logged once, 30 s after a signal whose Device opened no session. It used to fire only when the retry checkpoint moved, which means the Device had connected.
- Tests moved to `tests/device_push/` (`harness.rs` shared with the OTA and authorization tests).
- Resolved by changing the spec and ADR 0145: the payload carries `at` (the trigger moment), so a retry signal differs from the retained copy of a handled one.
