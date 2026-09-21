# 02 — Cấu trúc codebase đề xuất

## 1. Cây thư mục

```text
xiaozhi-lite-rs/
├── Cargo.toml
├── config.example.toml
├── README.md
├── docs/
├── scripts/
├── src/
│   ├── main.rs
│   ├── app.rs
│   ├── config.rs
│   ├── error.rs
│   │
│   ├── protocol/
│   │   ├── mod.rs
│   │   ├── client.rs
│   │   ├── server.rs
│   │   ├── audio.rs
│   │   └── mcp.rs
│   │
│   ├── transport/
│   │   ├── mod.rs
│   │   ├── http.rs
│   │   ├── ota.rs
│   │   └── websocket.rs
│   │
│   ├── session/
│   │   ├── mod.rs
│   │   ├── actor.rs
│   │   ├── event.rs
│   │   ├── state.rs
│   │   └── turn.rs
│   │
│   ├── audio/
│   │   ├── mod.rs
│   │   ├── codec.rs
│   │   ├── opus.rs
│   │   ├── vad.rs
│   │   ├── segmenter.rs
│   │   └── pacer.rs
│   │
│   ├── speech_output/
│   │   ├── mod.rs
│   │   ├── command.rs
│   │   └── worker.rs
│   │
│   ├── providers/
│   │   ├── mod.rs
│   │   ├── traits.rs
│   │   ├── asr/
│   │   ├── llm/
│   │   └── tts/
│   │
│   ├── dialogue/
│   │   ├── mod.rs
│   │   └── history.rs
│   │
│   ├── tools/
│   │   ├── mod.rs
│   │   └── device_mcp.rs
│   │
│   └── telemetry/
│       ├── mod.rs
│       └── metrics.rs
│
└── tests/
    ├── fixtures/
    ├── ws_protocol.rs
    ├── vad.rs
    ├── asr_contract.rs
    ├── llm_stream.rs
    ├── tts_stream.rs
    └── e2e_voice.rs
```

## 2. Trách nhiệm từng layer

### `protocol/`

Chỉ chứa type wire-format và serialize/deserialize. Không gọi network, AI hoặc state session.

### `transport/`

- Axum routes.
- WebSocket upgrade.
- Parse handshake headers.
- Spawn reader/writer.
- Chuyển dữ liệu thành `SessionEvent`.

Transport không quyết định conversation logic.

### `session/`

Đây là application core. `SessionActor` nhận event và quyết định bước tiếp theo.

### `audio/`

Codec, VAD adapter, text segmentation và pacing. Các thuật toán audio không biết WebSocket.

### `speech_output/`

`SpeechOutput` nhận `SpeechSegment`, `FinishInput` hoặc `Cancel` từ actor và trả `SpeechOutputEvent`. Implementation sở hữu TTS provider stream, normalize/resample, Opus encode và pacing; nó không giữ WS sender. Actor chuyển event hợp lệ thành outbound message theo một thứ tự duy nhất.

### `providers/`

Trait và adapter cho external AI services. Mỗi provider phải có contract test.

### `dialogue/`

Quản lý short-term conversation history và token/message limits.

### `tools/`

Device MCP adapter parse/serialize JSON-RPC và convert schema. Actor sở hữu tool registry, request ID và pending waiter; adapter chỉ nhận `McpCommand` và trả `McpResponse`.

## 3. Quy tắc dependency

Không được tạo dependency kiểu:

```text
ASR provider -> SessionActor
TTS provider -> WebSocket
LLM provider -> MCP transport trực tiếp
VAD -> Dialogue
SpeechOutput -> WebSocket
```

Thay vào đó mọi output phải quay về actor qua event.

## 4. Naming conventions

- Wire DTO: `ClientHello`, `ListenMessage`, `ServerHello`, `TtsStateMessage`.
- Domain event: `SessionEvent::AsrFinal`, `SessionEvent::LlmDelta`.
- Provider input/output: `AsrRequest`, `AsrResult`, `TtsRequest`, `AudioChunk`.
- Worker command/event: `SpeechOutputCommand`, `SpeechOutputEvent`, `McpCommand`, `McpResponse`.
- ID: `SessionId`, `GenerationId`, `McpRequestId` nên dùng newtype nếu codebase lớn dần.

## 5. Public API discipline

Mỗi module chỉ export type cần thiết qua `mod.rs`. Không dùng `pub` tràn lan.

Mục tiêu: thay implementation bên trong mà không đổi caller.
