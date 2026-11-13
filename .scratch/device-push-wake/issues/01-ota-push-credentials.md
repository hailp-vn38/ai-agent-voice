# 01: OTA cấp thông tin Device Push Channel cho Device đã đăng ký

**What to build:** Module `device_push` cùng mục cấu hình của nó, và khối `push` trong phản hồi OTA. Sau ticket này một Device đã đăng ký gọi OTA nhận đủ thông tin để kết nối broker.

**Blocked by:** Không.

Status: resolved

Spec: [device-push-wake](../spec.md) · [ADR 0145](../../../docs/adr/0145-device-push-channel-mqtt-wake-signal.md)

- [x] Mục cấu hình Device Push: bật/tắt (mặc định tắt), endpoint công khai, địa chỉ broker nội bộ, secret (đặt được qua biến môi trường), keepalive. Bật mà thiếu trường bắt buộc thì validation từ chối khởi động; cập nhật file cấu hình mẫu.
- [x] Module `device_push` tính Device Key, topic `signal`, client ID và mật khẩu HMAC theo quy ước trong spec; được dựng trong `AppState` chỉ khi tính năng bật.
- [x] Phản hồi OTA có khối `push` (`endpoint`, `client_id`, `username`, `password`, `keepalive`, `topic`) khi Device đã đăng ký và tính năng bật.
- [x] Không có khối `push` khi Device chờ đăng ký, bị chặn, hoặc tính năng tắt; phản hồi lúc tắt giống hệt hiện tại.
- [x] Phản hồi OTA không bao giờ có khoá `mqtt`; có test khẳng định điều này.
- [x] Device ID chứa `/`, `+`, `#` vẫn cho topic và client ID hợp lệ.
- [x] Test ở tầng HTTP của ứng dụng, theo mẫu các test đăng ký Device hiện có; chạy qua script kiểm thử của repo.

## Comments

- Implemented: `[device_push]` config (+ `VOICE_DEVICE_PUSH_SECRET`, validation, config.example.toml), `services::device_push` (Device Key, topic, client ID, HMAC `v1:{device_id}` password via ring), `AppState.device_push`, OTA `push` block for registered devices only. Tests: `crates/vela-server/tests/device_push_ota.rs` (HTTP) and a config unit test. Pre-existing unrelated failure: `background_http` (`unknown field admin_token`).
