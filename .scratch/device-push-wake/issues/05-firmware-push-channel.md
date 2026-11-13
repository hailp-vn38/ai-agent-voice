# 05: Firmware giữ Device Push Channel và mở phiên khi có Wake Signal

**What to build:** ESP32 đọc khối `push` từ OTA, giữ kết nối MQTT, và mở Voice Session không bật mic khi nhận Wake Signal.

**Blocked by:** 01: OTA cấp thông tin Device Push Channel cho Device đã đăng ký; 03: Mosquitto trong các bộ compose.

Status: ready-for-agent

Spec: [device-push-wake](../spec.md) · [ADR 0145](../../../docs/adr/0145-device-push-channel-mqtt-wake-signal.md)

- [x] Đọc `firmware/esp32/AGENTS.md` trước; làm trong cây firmware thật, build thật.
- [x] Parse khối `push` của phản hồi OTA vào vùng cấu hình riêng; không đụng tới xử lý khoá `mqtt` và `websocket`. Không có khối `push` thì Device chạy như hiện tại.
- [x] Lớp Device Push Channel tách khỏi tầng giao thức thoại: kết nối, subscribe topic được cấp, tự nối lại có backoff. Xác nhận client MQTT của bo dùng TLS trên cổng production; ghi kết quả vào Comments.
- [ ] Wake Signal hợp lệ, chưa hết hiệu lực, khi không có phiên: mở kênh âm thanh WebSocket với hello khai báo reminders, không bật mic, không vào trạng thái nghe.
- [ ] Bỏ qua tín hiệu sai `v`, sai `type`, thiếu `id`/`at`, đã quá `expires_at`, hoặc khi đang có phiên.
- [ ] Ngắt Device Push Channel khi mở Voice Session và nối lại khi phiên đóng.
- [ ] Broker không truy cập được: Device khởi động, wake word và trò chuyện vẫn bình thường.
- [ ] Log phần cứng thật cho: kết nối, nhận tín hiệu retained sau khi khởi động lại, mở phiên do Wake Signal.

## Comments

Branch work (firmware/esp32): `main/push/` (channel + pure decision header), OTA `push` parsing into NVS namespace `push` (stale grant erased when block absent), `Protocol::SetHelloReminders`, wiring in `Application`.

Verified: host test `python3 -m unittest scripts.tests.test_push_wake_signal` (version/type/expiry/busy decisions, retained-replay guard); real build `python3 scripts/build.py bread-compact-wifi --name bread-compact-wifi` with ESP-IDF v6.1-rc1 succeeded (esp32s3, app 36% free).

TLS by port (checked in `78__esp-ml307` 3.7.x source): both `EspMqtt` (Wi-Fi) and `Ml307Mqtt` (modem) enable TLS only when port == 8883 (Wi-Fi uses the cert bundle). The channel defaults to 8883 when the endpoint has no port; the production endpoint must therefore use 8883.

Not verified (needs hardware or a live broker): wake-opened session actually connecting, retained signal after reboot, no-mic behavior on a real Device, broker-unreachable boot, hardware logs. Boxes for these stay unchecked. The reminder playback/ACK path is ticket 06; here the session is opened and left idle (no listening). Caveats: `OTA` sets the system clock with the timezone offset added, so the expiry check can be skewed by that offset; the replay guard is in-RAM per boot; wake session on a voice-MQTT deployment declares nothing (hello flag only applies to the WebSocket protocol).

Review follow-up (2026-11-13), host tests and real build only, nothing run on hardware:

- A Wake Signal is no longer recorded as handled when it arrives. `wake_signal::PendingWake` keeps it until a wake session ran its course; a failed open or a dropped session is retried with backoff (5 s doubling to 60 s) until `expires_at`.
- Replay is decided by identity (`id`, `at`) instead of a time window after subscribe: a retained copy of a handled signal is ignored, a retry (new `at`) is not. Signals without `id` or `at` are ignored.
- Expiry is judged in UTC: `Ota::ClockUtcOffsetMs()` removes the timezone offset OTA adds to the system clock.
- `reminder_playback.h` moved to `main/reminder/`.
