# Voice Reference Client

`voice-reference-client` là một **Voice Protocol Client** độc lập bằng Rust. Nó dùng OTA discovery để lấy WebSocket URL và token, tự đặt các header V1 bắt buộc, rồi chạy các bước conformance mà không cần ESP32.

Nó không giả lập AI, TTS hoặc firmware. Mục đích của CLI là xác nhận ranh giới OTA/WebSocket và raw binary của Compatibility Profile `voice-ws-v1-baseline`.

## Chuẩn bị

Chạy server cục bộ với cấu hình mẫu trong một terminal:

```bash
VOICE_AGENT_CONFIG=config.example.toml cargo run -p voice-agent-server
```

Các lệnh dưới đây dùng OTA endpoint của server đó:

```bash
--ota http://127.0.0.1:8000/voice/ota/
```

CLI lấy `websocket.url` và `websocket.token` từ OTA response. Khi token không rỗng, nó tự gửi `Authorization: Bearer <token>`; không ghi token ra output hoặc file.

Mặc định `Device-Id` là `reference-client-01` và `Client-Id` là `reference-client`. Có thể thay đổi chúng khi cần kiểm thử session metadata:

```bash
cargo run -p voice-reference-client -- \
  --ota http://127.0.0.1:8000/voice/ota/ \
  --device-id reference-device-02 \
  --client-id conformance-run-02 \
  handshake
```

## Lệnh conformance

### Handshake

Gửi ClientHello V1 với uplink Canonical Audio Profile (Opus, 16 kHz, mono, 60 ms) và in ServerHello nhận được:

```bash
cargo run -p voice-reference-client -- \
  --ota http://127.0.0.1:8000/voice/ota/ handshake
```

### Manual listen

Sau hello, gửi `listen:start` rồi `listen:stop`:

```bash
cargo run -p voice-reference-client -- \
  --ota http://127.0.0.1:8000/voice/ota/ listen
```

### Gửi raw Opus uplink

`send-opus` đọc nguyên bytes của một file và gửi chúng trong WebSocket binary frame sau `listen:start`:

```bash
cargo run -p voice-reference-client -- \
  --ota http://127.0.0.1:8000/voice/ota/ \
  send-opus /absolute/path/to/opus-packet.bin
```

Ở Phase 1, server mới forward raw payload sang seam session và chưa decode Opus. Vì vậy file phải là fixture packet của conformance test; CLI không encode PCM thành Opus.

### Xác nhận raw binary downlink

`receive-binary` hoàn tất hello rồi chờ packet binary kế tiếp. Lệnh chỉ pass nếu payload khớp chính xác fixture hex:

```bash
cargo run -p voice-reference-client -- \
  --ota http://conformance-peer/voice/ota/ \
  receive-binary --expected-hex 00ff10
```

Lệnh này dùng với conformance peer có khả năng gửi downlink. Server Phase 1 cục bộ chưa có TTS/audio producer nên không tự phát packet downlink; đây không phải wire message test-only và không thay đổi protocol sản phẩm.

### Protocol-fault cases

Hai case hiện có kiểm tra fail-closed ở `AwaitHello`:

```bash
cargo run -p voice-reference-client -- \
  --ota http://127.0.0.1:8000/voice/ota/ \
  protocol-test --case binary-before-hello

cargo run -p voice-reference-client -- \
  --ota http://127.0.0.1:8000/voice/ota/ \
  protocol-test --case invalid-audio-profile
```

Cả hai cần nhận WebSocket close code `1002`. Danh sách case của CLI sẽ được mở rộng cùng protocol-conformance suite; xem [WebSocket testing](testing/01-ws.md) để biết toàn bộ contract bắt buộc.

## Xác minh CLI

```bash
cargo run -p voice-reference-client -- --help
cargo test -p voice-reference-client
```

Test peer tích hợp khởi tạo OTA + WebSocket peer độc lập, gửi ServerHello rồi raw binary `00 ff 10`, và xác nhận Reference Client nhận đúng từng byte.
