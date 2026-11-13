# 06: Firmware phát Reminder và gửi Playback ACK

**What to build:** Trong phiên do Wake Signal mở (và cả phiên thường), ESP32 phát Direct Speech của Reminder, gửi Playback ACK sau khi audio sink drain, chờ kết quả rồi đóng phiên.

**Blocked by:** 05: Firmware giữ Device Push Channel và mở phiên khi có Wake Signal.

Status: ready-for-agent

Spec: [device-push-wake](../spec.md) · [ADR 0145](../../../docs/adr/0145-device-push-channel-mqtt-wake-signal.md)

- [x] Hello khai báo khả năng reminders; firmware cũ không gửi thì server không gửi control mới.
- [x] Phát âm thanh Direct Speech có nguồn gốc Reminder qua đường phát hiện có.
- [x] Kết thúc luồng mạng không phải là đã phát: chỉ gửi `reminder.played` đúng Reminder và attempt sau khi bộ đệm decode và output đã drain.
- [x] Lỗi decode, lỗi output hoặc bị huỷ giữa chừng gửi `reminder.failed`.
- [x] Chờ `delivery_result` đúng attempt rồi mới đóng phiên do Wake Signal mở; có thời gian chờ tối đa. Nhiều Reminder đến hạn được phát tuần tự trong cùng phiên.
- [x] Phiên do Wake Signal mở mà server không phát gì thì tự đóng sau thời gian chờ và Device quay lại chờ wake word.
- [x] ACK trùng an toàn; không cache lịch, không ghi NVS cho Reminder, không hẹn giờ RTC.
- [ ] Bằng chứng phần cứng thật: tạo Reminder trên giao diện quản trị khi Device rảnh → Wake Signal → phiên → phát → ACK → Reminder `completed`.

## Comments

## Comments

Branch work (firmware/esp32): `main/push/reminder_playback.h` (pure attempt tracker, host-tested by `scripts/tests/test_reminder_playback.py`), wiring in `Application` (tts start/stop with `source=reminder`, `reminder` `delivery_result`, playback-drained, clock-tick timeouts), `Protocol::SendReminderEvent`, hello now always declares `features.reminders` (WebSocket). Protocol taken from `crates/vela-server` `session_protocol.rs` and fixture `voice_reminders_protocol_v1.json` (played/failed/delivery_result, failure reasons `output_unavailable|playback_aborted|decode_failed`).

Verified: host tests (`python3 -m unittest scripts.tests.test_push_wake_signal scripts.tests.test_reminder_playback`, 7 OK); real build `python3 scripts/build.py bread-compact-wifi --name bread-compact-wifi` with ESP-IDF v6.1-rc1 succeeded (36% app free). Boxes ticked cover logic and code paths only, not behavior on a Device.

Not verified / hardware-only: the last box (real Device end to end). All firmware behavior is unexercised on hardware, including that `IsPlaybackIdle()` really means the DAC/DMA output drained. Known gaps: `decode_failed` and `output_unavailable` are not raised from the audio service (it exposes no error callback); only `playback_aborted` (AbortSpeaking) and `output_unavailable` (network/main error event) are sent. A channel that closes mid-playback cannot send `failed` (the server must treat it as uncertain). Timeouts (empty wake session 10 s, delivery_result 8 s, 3 s grace between Reminders) are untuned constants. Status stays ready-for-agent because of the hardware box.

Review follow-up (2026-11-13), host tests and real build only, nothing run on hardware:

- `decode_failed` is now sent: `AudioServiceCallbacks::on_decode_error` fires when a received packet cannot be decoded.
- A wake session the user starts talking in becomes theirs (`Tracker::ReleaseToUser` on entering listening) and no longer closes itself.
- After a failed playback the server's retry checkpoint produces a new Wake Signal (new `at`), which the Device now accepts even though it handled the earlier one.
