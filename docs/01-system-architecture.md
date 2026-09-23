# 01 — Kiến trúc hệ thống

## 1. Context diagram

```mermaid
flowchart LR
    CLIENT[Voice Protocol Client] -->|HTTP OTA| HTTP[Axum HTTP]
    ESP <-->|WebSocket JSON + Opus| WS[WebSocket Transport]
    WS --> ACTOR[SessionActor]
    ACTOR --> VAD[VAD]
    ACTOR --> ASR[ASR Provider]
    ACTOR --> LLM[LLM Provider]
    ACTOR --> OUT[SpeechOutput]
    OUT --> TTS[TTS Provider]
    OUT --> WS
    ACTOR --> MCP[Device MCP Adapter]
```

## 2. Runtime tasks cho mỗi session

```mermaid
flowchart TB
    R[WS Reader] -->|SessionEvent| A[SessionActor]
    A -->|control + audio| W[WS Writer]
    A -->|PCM while Listening| V[VAD Worker]
    V -->|SpeechStart/SpeechEnd| A
    A -->|PCM stream + lifecycle| S[ASR Worker]
    S -->|AsrPartial/AsrFinal| A
    A -->|Start generation| L[LlmRuntime]
    L -->|bounded permit| LT[LLM stream task]
    LT -->|Delta/ToolCall| A
    A -->|SpeechOutputCommand| O[SpeechOutput]
    O -->|bounded command/event| T[TtsWorkerRuntime]
    T -->|native inference| P[TTS Provider]
    O -->|SpeechOutputEvent| A
    A -->|control + audio| W
    A -->|McpCommand| M[Device MCP Adapter]
    M -->|McpResponse| A
```

## 3. Ownership model

`SessionActor` sở hữu:

- `session_id`
- `device_id`, `client_id`
- `SessionPhase`
- Canonical Audio Profile đã validate
- listen mode
- dialogue buffer của Voice Session
- current `generation_id`
- current turn cancellation token
- MCP tool registry / pending requests
- quyền tăng `generation_id` và cập nhật `GenerationGate`

Các task khác nhận bản sao dữ liệu bất biến hoặc message DTO. Không giữ mutable reference trực tiếp đến actor state.

`SpeechOutput` là Module sâu, không phải một đường gửi WS thứ hai. Interface của nó chỉ nhận segment, tín hiệu input đã hết và cancel; implementation che giấu TTS stream, normalize/resample, Opus, queue và pacing. Nó trả event về actor; chỉ actor tạo `OutboundMessage`.

`LlmRuntime` chỉ cấp global permit, tạo task streaming theo Voice Session/generation và route event bounded; nó không giữ provider session mutable. `TtsWorkerRuntime` mới sở hữu native mutable inference, cancellation acknowledgement và quarantine.

## 4. Dependency direction

```text
main/app
   |
   +--> transport/http
   +--> transport/ws
   +--> session
   +--> providers implementations

session
   +--> protocol DTO
   +--> provider traits
   +--> audio abstractions
   +--> dialogue
   +--> tools abstractions

provider implementations
   +--> local inference runtimes hoặc reqwest / external APIs

protocol
   +--> serde only
```

Quy tắc: `protocol`, `session`, `dialogue` không được import implementation cụ thể của OpenAI, Whisper, FishSpeech, v.v.

## 5. State machine

```mermaid
stateDiagram-v2
    [*] --> AwaitHello
    AwaitHello --> Ready: client hello accepted
    Ready --> Listening: listen:start
    Listening --> Processing: utterance end + Active Turn permit
    Processing --> Speaking: first TTS output
    Speaking --> Ready: TTS stop (manual)
    Speaking --> Listening: TTS stop (auto/realtime)
    Speaking --> Listening: AEC-safe SpeechStart (barge-in)
    Processing --> Ready: abort/failure (manual)
    Processing --> Listening: abort/failure (auto)
    Ready --> Closed: disconnect
    Listening --> Closed: disconnect
    Processing --> Closed: disconnect
    Speaking --> Closed: disconnect
```

State phục vụ logging/validation. Pipeline async vẫn sử dụng generation id để chống stale output.

## 6. Reliability rules

- Provider timeout riêng cho ASR/LLM/TTS/MCP.
- `AsrStreamLease` được acquire khi recognition stream bắt đầu và release sau ASR final, cancel hoặc lỗi; `Active Turn` permit được acquire ở utterance terminal boundary, trước ASR finalization, không chờ queue vô hạn, và release ở mọi terminal path.
- `abort` là interruption explicit. `listen:start` chỉ arm/reset capture/VAD cycle và không tăng turn generation. Phase 5 chỉ route microphone khi `Speaking` vào VAD Barge-in Watch cho `Auto`/`Realtime` đã arm, client `features.aec=true` và cả hai config trust/enable đều bật; `Manual` không acoustic barge-in. `Realtime` giữ watch xuyên `Processing`/`Speaking`, còn Auto chỉ watch khi cycle đã arm.
- Transport idle dựa trên WebSocket RX/TX hợp lệ hai chiều; conversation idle tắt trong V1.
- Telemetry dùng Trace Session ID ngẫu nhiên; không log/persist nội dung audio, transcript, prompt/response, tool payload hay secret.
- Mọi task con gắn với session cancellation root.
- Turn cancellation không đóng WS; session cancellation mới đóng tất cả.
- Mọi queue ingress, worker command, audio và outbound đều bounded; mỗi queue có capacity và overload policy được cấu hình.
- WS writer nhận bản sao chỉ-đọc của `GenerationGate`; actor cập nhật gate trước urgent `tts:stop` để writer drop mọi JSON/audio stale dù đã nằm trong queue. Urgent lane ưu tiên normal control và audio; không admission được interrupt stop sau `tts:start` là session-integrity failure, fail-closed không phụ thuộc retry queue.
- `AwaitHello` fail closed: chỉ ClientHello canonical hợp lệ mới tạo Voice Session; lỗi application handshake đóng 1002.
- Sau handshake, parser ignore unknown field/type và invalid application message không thay đổi state hay đóng session.
- Frame vượt size cap đóng 1009 trước application parse; WebSocket framing/UTF-8 fault do transport library xử lý.
- `Drop`/cleanup phải đóng pending MCP waiters và provider streams.
- WS writer sở hữu control và audio queue riêng, cả hai bounded; control hợp lệ có ưu tiên. Audio phải mang generation và bị kiểm tra lại ngay trước send.
