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
    AsrPartial(AsrPartial),
    AsrFinal(AsrResult),
    Llm(LlmEvent),
    SpeechOutput(SpeechOutputEvent),
    ProviderError { source: ProviderKind, error: ProviderError },
}
```

Parser biến listen thành semantic variants `ListenStart { mode: ListeningMode }`, `ListenStop` và `ListenDetect { text }`, không để actor diễn giải tổ hợp string/optional field. Chỉ Start yêu cầu mode. Phase 5 hỗ trợ `Manual`, `Auto` và `Realtime`: `listen:start` chỉ arm/reset capture/VAD cycle, không cancel delivery hay tăng generation. `abort` là control interruption explicit; Acoustic Barge-in chỉ bắt đầu từ `SpeechStart` đủ điều kiện. Missing/invalid mode là invalid application message và `ListenStop` ngoài Manual `Listening` là valid wrong-state message: trace/ignore, không mutate turn.

### Invariant

Actor phải bỏ mọi `SessionEvent::Turn` có `generation != current_generation`, trừ event cleanup/telemetry. VAD semantic event còn phải mang `VadCycleId` và chỉ được nhận khi cycle hiện hành; lease cleanup acknowledgement vẫn xử lý khi stale. Mọi output async thuộc một turn phải đi qua biến thể này.

## 2. Provider traits

Conceptual contract:

```rust
pub trait VadProvider: Send + Sync {
    fn open(&self) -> Result<Box<dyn VadSession>, VadError>;
}

pub trait VadSession: Send {
    fn push_pcm(&mut self, pcm: &[f32]) -> Result<Vec<VadFrame>, VadError>;
    fn reset(&mut self) -> Result<(), VadError>;
}

pub trait AsrProvider: Send + Sync {
    fn open(&self, request: AsrStartRequest) -> Result<Box<dyn AsrSession>, AsrError>;
}

pub trait AsrSession: Send {
    fn push_pcm(&mut self, pcm: &[f32]) -> Result<Vec<AsrEvent>, AsrError>;
    fn finish(&mut self) -> Result<AsrResult, AsrError>;
    fn cancel(&mut self);
}

pub trait LlmProvider: Send + Sync {
    fn stream(&self, req: LlmRequest) -> Result<LlmStream, LlmError>;
}

pub trait TtsProvider: Send + Sync {
    fn synthesize(&self, req: TtsRequest) -> Result<TtsStream, TtsError>;
}
```

`VadProvider` trả `VadProbability` model-level gồm probability và contiguous `[start_sample, end_sample)`; Silero session giữ recurrent state cùng 64-sample model context. `VadSegmenter` ở core chỉ sở hữu hysteresis, candidate onset, `min_speech_ms`, `end_silence_ms` và endpoint semantics theo sample timeline; actor/core audio capture sở hữu hard-bounded PCM retention và onset-relative pre-roll. `AsrProvider` canonical là streaming: adapter offline/HTTP tương lai có thể buffer trong `push_pcm()` rồi infer ở `finish()`, nhưng không đổi contract. Trait không nhận `SessionActor`, WebSocket sender hoặc config global mutable.

Typed provider configuration chỉ chọn compiled adapter, Logical Model Identity và runtime option. Model Preparation resolve manifest trước bind rồi inject `ResolvedModel` theo artifact role vào Provider Factory; provider không nhận direct file path từ config và không được tự acquire artifact.

Phase 3 default dùng `silero_onnx` local Rust cho VAD và `zipformer_sherpa` local Rust cho ASR; không có Python sidecar hoặc HTTP ASR trong baseline. Provider event chỉ quay về actor qua bounded worker boundary và phải mang Voice Session identity cùng generation khi thuộc turn.

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

`Started` chỉ phát khi AudioPacket hợp lệ đầu tiên đã sẵn sàng; actor chuyển nó thành `tts:start` rồi enqueue packet đó. `AudioPacket` là kết quả đã được pace, không phải một queue hay sender để caller điều khiển. `Drained` chỉ phát sau `FinishInput`, khi mọi segment đã hoàn tất và packet cuối đã qua pacer. Actor chỉ gửi `tts:stop` của luồng bình thường khi nhận event này. Non-empty TTS input có provider completion nhưng zero valid AudioPacket là `Failed(tts_empty_audio)`. TTS native mutable inference chạy trong `TtsWorkerRuntime` bounded; safe-point cancel không thay đổi yêu cầu GenerationGate drop output stale và cleanup acknowledgement trước khi release slot. Per generation chỉ có một active synthesis; pending ordinal liên tục/bounded, còn `Drained` cần `FinishInput`, pending rỗng, no active synthesis và final audio đã paced.

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

Actor là producer duy nhất của outbound message. WS writer nhận ba lane bounded `urgent > normal control > audio` và giữ `GenerationGate` read-only để drop mọi `Turn` không còn là generation hiện tại, gồm cả JSON lẫn audio. Actor cập nhật gate trước rồi enqueue urgent `SessionControl(tts:stop)`; packet/control turn cũ đã xếp hàng bị drop trước stop. `tts:start` phải được writer gửi trước AudioPacket đầu tiên của generation. Nếu urgent stop không admission được sau `Started`, actor fail-closed Voice Session bằng root cancellation/writer shutdown escape path, không silent ignore hay retry vô hạn.

Actor giữ `tts_started`/`tts_stopped` theo generation: failure trước `Started` không gửi control pair; failure sau `Started` invalidate generation, cancel pipeline, drop audio stale rồi enqueue chính xác một `tts:stop`. Sau stop, writer không được gửi binary audio của generation đó; failure path không commit Delivered Assistant Response.

`AsrPartial` là internal-only: có thể coalesce nhưng không đi vào dialogue, không khởi động LLM và không tạo WebSocket message. Chỉ `AsrFinal` current-generation với `text.trim()` non-empty mới commit user utterance, enqueue đúng một message V1 hiện có `{"session_id":"...","type":"stt","text":"..."}`, rồi bắt đầu LLM. Final rỗng, lỗi, cancel hoặc stale không gửi `stt`; V1 không thêm `stt_partial`, `asr_partial`, `asr_final`, `vad_start`, `vad_stop` hay field wire mới.

## 5. Audio và queue invariants

- Uplink V1: raw Opus packet.
- Uplink Canonical Audio Profile: raw Opus 16 kHz, mono, 60 ms; validate ở hello trước Ready.
- PCM canonical ở core là `Pcm16Mono`; `UplinkPcmFrame`, `DownlinkPcmFrame` và `UplinkAudioUtterance` bọc type này để biểu thị boundary semantic. Provider VAD/ASR nhận conversion `PcmF32Mono` có sample rate tường minh, không nhận `Vec<u8>`. `ZeroTtsProvider` cũng chỉ trả `PcmF32Mono`, nhưng V1 contract của nó là 48 kHz mono; `SpeechOutput` chuyển nó về `Pcm16Mono` 24 kHz. Constructors frame chỉ nhận đúng 960 samples uplink hoặc 1.440 samples downlink; `UplinkAudioUtterance` có độ dài biến thiên trong giới hạn capture.
- Phase 3 đưa local inference qua worker boundary bounded để không block Tokio executor. `AsrStreamLease` được lấy lúc `SpeechStart` (Auto) hoặc `listen:start` (Manual); `Active Turn` permit chỉ được lấy ở `SpeechEnd` hoặc `listen:stop`, trước `AsrSession.finish()`. Không có permit thì cancel stream và release lease, không xếp chờ.
- Decoder trả `DecodeOutcome::Frame(UplinkPcmFrame)` hoặc `DecodeOutcome::Dropped(AudioFrameDropReason)` cho packet rỗng, packet vượt `MAX_UPLINK_OPUS_PACKET_BYTES = 4.000`, decode lỗi và sample count sai. `AudioFrameDropReason` phân biệt `EmptyPacket`, `PacketTooLarge`, `DecodeError` và `InvalidSampleCount`; đây là local frame fault expected, actor chỉ ghi tracing metadata privacy-safe rồi tiếp tục. 4.000 bytes uplink là V1 implementation policy, không phải giới hạn format Opus. `DownlinkOpusEncoder` chỉ nhận `DownlinkPcmFrame`.
- `DownlinkOpusEncoder` dùng profile implementation constant: VoIP, 32 kbps, VBR/constrained VBR bật, DTX/FEC tắt, packet-loss percent 0 và complexity 10. Không lấy các controls này từ config ở Phase 2. Encoder luôn dùng `DOWNLINK_ENCODE_BUFFER_BYTES = 4.000`, tách cả `MAX_UPLINK_OPUS_PACKET_BYTES` lẫn `websocket.max_frame_bytes`; `encode(DownlinkPcmFrame)` trả `Result<OpusPacket, AudioCodecError>`; encoder error, packet rỗng và packet lớn hơn transport cap là internal delivery failure, không phải local frame drop và không đóng WS 1009.
- Downlink Canonical Audio Profile: Opus 24 kHz, mono, 60 ms. Provider PCM có thể normalize/resample nội bộ về profile này.
- Pacer không được nhận unbounded queue.
- Ingress WS, command của VAD/ASR/LLM/TTS và outbound đều phải bounded, có capacity và hành vi khi đầy. VAD ingress đầy không được drop PCM rồi tiếp tục semantic timeline: affected Voice Session fail-closed; ASR ingress đầy là controlled recognition failure, không drop ngẫu nhiên PCM rồi coi transcript hợp lệ; TTS producer bị backpressure; outbound đầy là lỗi turn có kiểm soát. `AutoPcmRetention` dùng cho Auto/Realtime/Barge-in có overwrite-oldest và capacity = `pre_roll + confirmation + vad_command_capacity * 960 + 960 + rechunk_slack`; snapshot phải diễn ra trước reset cycle.

## 6. Dialogue invariants

- `system` luôn ở đầu logical prompt.
- `DialogueHistory` thuộc Voice Session, bounded bởi `llm.max_history_messages` và RAM-only; actor chỉ gọi `commit_user`, còn eviction thuộc history. Commit user utterance sau ASR final non-empty hợp lệ, trước enqueue STT, kể cả khi outbound sau đó lỗi.
- Chỉ commit assistant response vào history sau `SpeechOutputEvent::Drained`; generated response bị cancel/lỗi không phải response đã delivered.
- Phase 4 luôn gọi LLM với `tools = None`; bất kỳ tool call nào là `llm_unexpected_tool_call`, fail generation, cancel SpeechOutput và không chạy MCP/retry. Audio đã deliver không thể thu hồi, nhưng không audio queued/stale nào được gửi tiếp.
- Từ Phase 6, với LLM-visible Tool, buffer toàn bộ LLM round; prose của round có tool call không được gửi vào SpeechOutput.
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
