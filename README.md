# voice-agent-server — Development Documentation

Bộ tài liệu kiến trúc và phát triển cho một voice-agent server Rust tối giản với WebSocket protocol v1.

## Mục tiêu

Dự án không port bất kỳ reference implementation nào theo kiểu 1:1. Thay vào đó, dự án định nghĩa **wire protocol + behavior cốt lõi** của riêng mình và xây kiến trúc Rust nhỏ, rõ ràng, dễ kiểm thử và dễ thay provider.

Core V1 phục vụ mọi **Voice Protocol Client** tuân thủ contract; firmware reference chỉ là nguồn đối chiếu đã pin, không phải loại client duy nhất.

Core V1:

- OTA/discovery endpoint.
- WebSocket protocol v1.
- Opus uplink/downlink.
- VAD + manual listen mode.
- ASR provider.
- Streaming LLM provider.
- TTS provider + Opus pacing.
- Short-term dialogue.
- Abort/barge-in/cancellation.
- Device MCP (`initialize`, `tools/list`, `tools/call`).

Không nằm trong V1: manager web/mobile, multi-user, database bắt buộc, MQTT/UDP gateway, RAG, voiceprint, billing/quota, plugin hot-load, long-term memory.

## Chạy nhanh Protocol V1

Khởi động server với cấu hình mẫu:

```bash
VOICE_AGENT_CONFIG=config.example.toml cargo run -p voice-agent-server
```

Sau đó xác nhận OTA → WebSocket → ClientHello bằng Voice Reference Client độc lập:

```bash
cargo run -p voice-reference-client -- \
  --ota http://127.0.0.1:8000/voice/ota/ handshake
```

CLI tự lấy WebSocket URL/token từ OTA response và thêm các header V1 cần thiết. Hướng dẫn cho `listen`, raw Opus uplink/downlink và protocol-fault cases ở [Voice Reference Client](docs/voice-reference-client.md).

Chạy các gate tự động hiện có:

```bash
./scripts/test-all.sh
```

## Thứ tự đọc

1. [`docs/00-overview.md`](docs/00-overview.md)
2. [`docs/01-system-architecture.md`](docs/01-system-architecture.md)
3. [`docs/02-codebase.md`](docs/02-codebase.md)
4. [`docs/03-module-contracts.md`](docs/03-module-contracts.md)
5. [`docs/04-configuration.md`](docs/04-configuration.md)
6. [`docs/05-development-workflow.md`](docs/05-development-workflow.md)
7. Các flow trong [`docs/flows/`](docs/flows/)
8. Chiến lược test trong [`docs/testing/`](docs/testing/)
9. ADR trong [`docs/adr/`](docs/adr/)
10. [Voice Reference Client](docs/voice-reference-client.md)

## Nguồn tham chiếu external

- Server: `xinnan-tech/xiaozhi-esp32-server`
- Firmware: `78/xiaozhi-esp32`

Chi tiết mapping sang code Rust đề xuất nằm tại [`docs/reference/source-map.md`](docs/reference/source-map.md).
