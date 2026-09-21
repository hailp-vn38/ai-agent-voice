# Reference source map

## Upstream snapshot dùng khi soạn tài liệu

- `xinnan-tech/xiaozhi-esp32-server`: `788f5301fdd60cc3a8ef74025bfeece9b82b94ce` (2026-09-21).
- `78/xiaozhi-esp32`: v2.5.0, `ac6deed3d8e75348475364bf40ad953c6cd48054`.

Compatibility Profile V1 chỉ lấy fixture từ firmware commit đã pin; `main` chỉ theo dõi upstream. Các flow protocol quan trọng được đối chiếu trực tiếp với firmware WebSocket implementation. Khi chủ động đổi compatibility baseline, cập nhật snapshot này trước rồi chạy lại protocol contract tests.

## Mapping sang Rust

| Reference | Ý nghĩa | Rust target |
|---|---|---|
| `xiaozhi-server/core/websocket_server.py` | WS accept/auth | `transport/websocket.rs` |
| `xiaozhi-server/core/connection.py` | Session state + orchestration | tách thành `session/*`, không port 1:1 |
| `core/handle/helloHandle.py` | hello/MCP init | `session/actor.rs`, `tools/device_mcp.rs` |
| `core/handle/textHandler/listenMessageHandler.py` | listen state | `protocol/client.rs` + actor |
| `core/handle/abortHandle.py` | abort | `session/turn.rs` |
| `core/handle/receiveAudioHandle.py` | VAD/audio to chat | `audio/vad.rs` + actor |
| `core/providers/asr/base.py` | ASR abstraction | `providers/traits.rs`, `providers/asr/*` |
| `core/providers/llm/openai/openai.py` | OpenAI-compatible streaming | `providers/llm/openai.rs` |
| `core/providers/tts/base.py` | text segmentation/TTS queues | `speech_output/*` che giấu `audio/segmenter.rs`, `providers/tts/*`, `audio/pacer.rs` |
| `core/handle/sendAudioHandle.py` | TTS state + pacing | `speech_output/*` event -> `session/actor.rs` -> WS outbound |
| `core/providers/tools/device_mcp/*` | Device MCP | `tools/device_mcp.rs` |
| `core/api/ota_handler.py` | OTA discovery | `transport/ota.rs` |
| `78/xiaozhi-esp32/docs/websocket_zh.md` | firmware protocol contract | `protocol/*` tests/fixtures |
| `78/xiaozhi-esp32/main/protocols/websocket_protocol.cc` | authoritative client behavior | WS integration tests |

## Những phần cố ý không port

- manager API/web/mobile.
- report worker/chat history upload.
- runtime config hot reload.
- Python plugin loader.
- multi-provider registry lớn.
- voiceprint/RAG/memory enterprise features.
- MQTT gateway/AEC path ở V1.
