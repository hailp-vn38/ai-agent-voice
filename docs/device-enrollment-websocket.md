# Thêm Device qua WebSocket

Baseline của thay đổi: dev-test `24502d33696f5d72c2993bcaf88263f39a3521bb`, đã
merge database/admission luôn bật. Không đưa lại database.enabled hoặc
database.devices.admission_enabled.

## Chuẩn bị và bật tính năng

```bash
python3 scripts/prepare-enrollment-assets.py
```

Cần espeak-ng/ffmpeg trên máy chuẩn bị asset; không cần chúng trên server sau khi
đã có WAV. Nghe lại clips trước deployment. Có thể thay bằng recording riêng theo
[asset contract](../assets/enrollment/README.md). Runtime không tải asset hoặc gọi
TTS provider từ mạng. Copy cả directory WAV khi deploy.

```toml
[database.devices]
auto_register = false

[database.devices.enrollment]
enabled = true
transport = "websocket"
code_ttl_seconds = 600
retention_seconds = 86400
cleanup_interval_seconds = 60
max_pending = 1000
ws_max_connections = 32
ws_timeout_seconds = 120
ws_poll_interval_ms = 2000
ws_prompt_repeat_seconds = 60
prompt_assets_dir = "assets/enrollment/vi-VN"

[api]
enabled = true
admin_token = "<deployment-admin-token>"
```

Enrollment default off; enabled mà không transport dùng websocket. Config cũ
muốn giữ OTA activation/polling đặt transport ota. Trong mode ota không cần WAV.

## Flow thiết bị và web

1. Device gọi GET/POST `/voice/ota/` với Device-Id/Client-Id. Unknown có enrollment
   còn TTL hoặc được cấp mới; trả websocket URL/token chung, không activation.
   Registered cũng nhận cấu hình này; blocked 403, DB lỗi 503.
2. Khi firmware mở WS `/voice/v1/`, server kiểm tra bearer và identity trước DB.
   Unknown được upgrade vào Enrollment Session, Registered đi qua voice admission.
3. ClientHello/ServerHello v1, rồi stt hiển thị mã. Server phát intro và sáu digit
   bằng raw Opus dưới một playback tts; ví dụ 042731 đọc cả chữ số 0 đầu. Uplink
   bị bỏ, không có ASR/LLM/provider/tools/transcript.
4. Trên web: Agent → Thêm thiết bị; xem/nghe mã trên thiết bị rồi nhập mã string,
   tên và Template (null theo Agent). API Admin claim hiện có không thay DTO.
5. Worker quan sát Device registered (claim hoặc manual-create), kết thúc playback,
   gửi thông báo rồi close 1000. Người dùng bấm/wake để mở WS mới, hoặc client có
   reconnect riêng; không giả định firmware tự mở lại ngay khi close.
6. Kết nối mới resolve Device → Agent → Template → Provider đúng revision. Runtime
   lỗi có thể trả 503; đó không phải claim thất bại. Claim không báo online.

Firmware có thể chỉ mở audio WS khi wake/bấm nút. Muốn nghe mã ngay khi boot cần
client chủ động mở WS sau OTA; server không thể phát audio qua socket chưa tồn tại.

## Wire trace

Sau ServerHello (24 kHz mono 60 ms), thứ tự là:

```json
{"type":"stt","session_id":"...","text":"Mã kết nối: 042731. Mở web, chọn Agent → Thêm thiết bị và nhập mã này."}
{"type":"tts","state":"start","session_id":"..."}
{"type":"tts","state":"sentence_start","session_id":"...","text":"Mã kết nối: 0 4 2 7 3 1"}
```

Tiếp theo mỗi WS binary là một raw Opus packet (không WAV/Ogg/MQTT header). Sau
packet cuối writer mới gửi:

```json
{"type":"tts","state":"stop","session_id":"..."}
```

stt là lời nhắc hệ thống tương thích client, không phải ASR/user history. Không cần
type activation custom. Cùng session_id trong hello và các message tts/stt.

## Timeout, replay và lỗi

- WS sống tối đa min(120 giây, thời gian mã còn lại), không reset bởi ping/audio.
  Reconnect lấy cùng mã nếu TTL còn; hết TTL lấy mã mới, không đổi mã giữa playback.
- listen sau cooldown 60 giây có thể đọc lại. abort dừng audio nhưng không consume
  hoặc cancel enrollment. Một playback/connection, queue và encode memory bounded.
- Header/bearer sai không cấp enrollment. Duplicate identity/auth header bị từ chối.
  Unknown khi feature off/mode ota giữ 403 tại WS; blocked trước upgrade 403.
- Blocked sau upgrade close 1008; DB unavailable close 1011; code expired close
  1000 với hướng dẫn mở lại. Oversized frame 1009, invalid/timeout hello 1002.
- Claim commit thành công vẫn tồn tại nếu WS disconnect. Không retry SQLite BUSY;
  không tự claim lại hoặc fallback provider defaults cho Unknown.
- Assets missing/invalid fail bootstrap; injected state không prepare assets sẽ
  không ready và Unknown WS 503, thay vì chỉ hiện text và bỏ âm thanh.

## Kiểm thử

```bash
cargo fmt --all -- --check
cargo test -p voice-agent-server --test device_enrollment_ws
cargo test -p voice-agent-server --test device_admission
cargo test -p voice-agent-server --test database_bootstrap
cargo test -p voice-agent-server --test ws_protocol
cargo test -p voice-agent-server services::device_enrollment
```

Tests dùng WAV synthetic, SQLite file-backed và HTTP/WS thật; kiểm tra leading
zero/PCM order trước encode, decode raw Opus 1440 samples/frame, stop sau audio,
claim/reconnect, auth/cap/expiry/abort và bootstrap assets failure. Chưa thay thế
smoke trên ESP32: xác minh màn hình/loa, sáu digit, close reset client và lần wake
tiếp theo dùng đúng Agent. Không cần model/credential thật cho deterministic gate.

Xem [ADR-0074](adr/0074-websocket-enrollment-session.md).
