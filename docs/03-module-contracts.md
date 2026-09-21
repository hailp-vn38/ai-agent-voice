# 03 — Module contracts và invariants

## 1. Session event contract

Ví dụ event model:

```rust
pub enum SessionEvent {
    ClientMessage(ClientMessage),
    ClientAudio(bytes::Bytes),
    Vad(VadEvent),
    Turn { generation: GenerationId, event: TurnEvent },
    McpResponse(McpResponse),
    Disconnected,
}

pub enum TurnEvent {
    AsrFinal(AsrResult),
    Llm(LlmEvent),
    SpeechOutput(SpeechOutputEvent),
    ProviderError { source: ProviderKind, error: ProviderError },
}
```

Parser biến listen thành semantic variants `ListenStart { mode: ListeningMode }`, `ListenStop` và `ListenDetect { text }`, không để actor diễn giải tổ hợp string/optional field. Chỉ Start yêu cầu mode. Ở Phase 2 chỉ `Manual` được hỗ trợ; `Auto` và `Realtime` vẫn parse được nhưng là unsupported application message, còn missing/invalid mode là invalid application message. Tất cả các trường hợp đó chỉ trace/ignore, không mutate phase, cancel turn hay reset capture. `ListenStop` chỉ finalize Manual Capture active; ở phase khác là valid wrong-state message nên trace/ignore.

### Invariant

Actor phải bỏ mọi `SessionEvent::Turn` có `generation != current_generation`, trừ event cleanup/telemetry. Mọi output async thuộc một turn phải đi qua biến thể này.

## 2. Provider traits

Conceptual contract:

```rust
#[async_trait]
pub trait AsrProvider: Send + Sync {
    async fn transcribe(&self, req: AsrRequest) -> Result<AsrResult, AsrError>;
}

pub trait LlmProvider: Send + Sync {
    fn stream(&self, req: LlmRequest) -> Result<LlmStream, LlmError>;
}

pub trait TtsProvider: Send + Sync {
    fn synthesize(&self, req: TtsRequest) -> Result<TtsStream, TtsError>;
}
```

Trait không nhận `SessionActor`, WebSocket sender hoặc config global mutable.

VAD V1 là implementation local bên trong `audio/`, không phải Seam provider. Chỉ tạo `VadProvider` khi có ít nhất hai Adapter cần hỗ trợ; tránh tạo một Interface giả định.

## 3. SpeechOutput contract

```rust
pub enum SpeechOutputCommand {
    Submit { generation: GenerationId, ordinal: u32, text: String },
    FinishInput { generation: GenerationId },
    Cancel { generation: GenerationId },
}

pub enum SpeechOutputEvent {
    Started { generation: GenerationId },
    SentenceStarted { generation: GenerationId, ordinal: u32, text: String },
    AudioPacket { generation: GenerationId, packet: bytes::Bytes },
    SegmentFinished { generation: GenerationId, ordinal: u32 },
    Drained { generation: GenerationId },
    Failed { generation: GenerationId, error: TtsError },
}
```

`Started` chỉ phát khi AudioPacket hợp lệ đầu tiên đã sẵn sàng; actor chuyển nó thành `tts:start` rồi enqueue packet đó. `AudioPacket` là kết quả đã được pace, không phải một queue hay sender để caller điều khiển. `Drained` chỉ phát sau `FinishInput`, khi mọi segment đã hoàn tất và packet cuối đã qua pacer. Actor chỉ gửi `tts:stop` của luồng bình thường khi nhận event này. Non-empty TTS input có provider completion nhưng zero valid AudioPacket là `Failed(tts_empty_audio)`.

## 4. Outbound contract

```rust
pub enum OutboundMessage {
    Turn {
        generation: GenerationId,
        payload: OutboundPayload,
    },
    SessionControl(ServerMessage),
    Close,
}

pub enum OutboundPayload {
    Json(ServerMessage),
    Audio(bytes::Bytes),
}
```

Actor là producer duy nhất của outbound message. WS writer nhận control và audio qua hai queue bounded riêng, ưu tiên control hợp lệ, và giữ `GenerationGate` read-only để drop mọi `Turn` không còn là generation hiện tại. Actor cập nhật gate trước rồi enqueue `SessionControl(tts:stop)`; packet cũ đã xếp hàng bị drop trước stop. `tts:start` phải được writer gửi trước AudioPacket đầu tiên của generation.

## 5. Audio và queue invariants

- Uplink V1: raw Opus packet.
- Uplink Canonical Audio Profile: raw Opus 16 kHz, mono, 60 ms; validate ở hello trước Ready.
- PCM internal chỉ có `Pcm16Mono(Vec<i16>)`; `UplinkPcmFrame`, `DownlinkPcmFrame` và `UplinkAudioUtterance` bọc type này để biểu thị boundary semantic, không lặp sample rate/channels và không để `Vec<i16>` lan trong domain. Constructors frame chỉ nhận đúng 960 samples uplink hoặc 1.440 samples downlink; `UplinkAudioUtterance` có độ dài biến thiên trong giới hạn capture.
- Phase 2 giữ xử lý audio theo thứ tự trong `SessionActor`: actor sở hữu private `UplinkOpusDecoder` và `ManualCapture` nhưng không biết type `opus2`, PCM storage hay capacity calculation. Chỉ tách audio worker khi profiling cho thấy decode/VAD làm block actor đáng kể.
- Decoder trả `DecodeOutcome::Frame(UplinkPcmFrame)` hoặc `DecodeOutcome::Dropped(AudioFrameDropReason)` cho packet rỗng, packet vượt `MAX_UPLINK_OPUS_PACKET_BYTES = 4.000`, decode lỗi và sample count sai. `AudioFrameDropReason` phân biệt `EmptyPacket`, `PacketTooLarge`, `DecodeError` và `InvalidSampleCount`; đây là local frame fault expected, actor chỉ ghi tracing metadata privacy-safe rồi tiếp tục. 4.000 bytes uplink là V1 implementation policy, không phải giới hạn format Opus. `DownlinkOpusEncoder` chỉ nhận `DownlinkPcmFrame`.
- `DownlinkOpusEncoder` dùng profile implementation constant: VoIP, 32 kbps, VBR/constrained VBR bật, DTX/FEC tắt, packet-loss percent 0 và complexity 10. Không lấy các controls này từ config ở Phase 2. Encoder luôn dùng `DOWNLINK_ENCODE_BUFFER_BYTES = 4.000`, tách cả `MAX_UPLINK_OPUS_PACKET_BYTES` lẫn `websocket.max_frame_bytes`; `encode(DownlinkPcmFrame)` trả `Result<OpusPacket, AudioCodecError>`; encoder error, packet rỗng và packet lớn hơn transport cap là internal delivery failure, không phải local frame drop và không đóng WS 1009.
- Downlink Canonical Audio Profile: Opus 24 kHz, mono, 60 ms. Provider PCM có thể normalize/resample nội bộ về profile này.
- Pacer không được nhận unbounded queue.
- Ingress WS, command của ASR/LLM/TTS và outbound đều phải bounded, có capacity và hành vi khi đầy. Uplink frame khi đầy bị drop + telemetry; TTS producer bị backpressure; outbound đầy là lỗi turn có kiểm soát.

## 6. Dialogue invariants

- `system` luôn ở đầu logical prompt.
- Commit user utterance sau ASR final hợp lệ.
- Chỉ commit assistant response vào history sau `SpeechOutputEvent::Drained`; generated response bị cancel/lỗi không phải response đã delivered.
- Với LLM-visible Tool, buffer toàn bộ LLM round; prose của round có tool call không được gửi vào SpeechOutput.
- Tool call/result phải theo đúng ordering của LLM API.
- History có đồng thời message limit và prompt token budget; eviction theo Exchange Atom cũ nhất, không tách tool call/result; system và current turn luôn giữ.
- Tool result phải sanitize và cap trước LLM context, có đánh dấu truncation nếu bị cắt.
- `TurnOutcome` là `Completed`, `CompletedSilent`, `Cancelled` hoặc `Failed`; ASR success nhưng `trim()` rỗng là `CompletedSilent`, không tạo dialogue message.

## 7. Error policy

| Lỗi | Hành vi |
|---|---|
| Malformed client JSON | log + ignore message |
| Opus decode failure một frame | drop frame |
| ASR timeout | stop current turn + báo lỗi ngắn |
| LLM timeout | cancel TTS pending + stop turn |
| TTS timeout | gửi `tts:stop` và trở lại listening |
| MCP timeout khi session khỏe | normalized tool error về LLM; terminal khi session/cancellation failure |
| WS disconnect | cancel toàn session |

Không `unwrap()` trên dữ liệu đến từ network hoặc provider.

Tool-level timeout/JSON-RPC error/`isError` khi session khỏe trở thành tool result đã sanitize cho LLM continuation. Disconnect, session replacement, root cancellation, shutdown, generation cancellation hay MCP routing mất session là terminal. Không automatic retry logical provider/tool operation.
