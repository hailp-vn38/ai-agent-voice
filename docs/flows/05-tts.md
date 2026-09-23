# Flow 05 — TTS, Opus và audio pacing

## 1. Input

`SpeechOutput` nhận **speakable text segment**, không nhận toàn bộ response bắt buộc. TTS provider là implementation bên trong Module này.

```rust
pub struct TtsRequest {
    pub generation: u64,
    pub text: String,
    pub voice: String,
    pub output_sample_rate: u32,
}
```

Adapter concrete V1 là `zerotts_onnx`, native Rust + ONNX. Provider chỉ infer và trả typed PCM cùng sample rate thực tế; `SpeechOutput` normalize/resample PCM về 24 kHz mono rồi Opus encode. Actor không thấy implementation hoặc wire format của provider.

`TtsWorkerRuntime` application-owned chạy ZeroTTS/ORT trên worker bounded, không trên Tokio executor. Nó sở hữu native mutable inference, command queue, timeout, cancel/cleanup acknowledgement và quarantine. `SpeechOutput` giữ semantic lifecycle; khi inference không preempt được, cancel có hiệu lực tại safe point kế tiếp nhưng output stale vẫn bị GenerationGate drop và slot chỉ reusable sau cleanup acknowledgement.

ZeroTTS V1 trả `PcmF32Mono` 48 kHz mono: codec stereo được adapter average/normalize trước boundary. `SpeechOutput` resample 48 kHz -> 24 kHz, convert f32 -> i16, rồi tạo đúng 1.440-sample `DownlinkPcmFrame`. Factory/warmup xác minh config và codec metadata đều 48 kHz, channel profile và voice dimensions khớp graph; thay đổi profile ở revision khác fail startup.

ZeroTTS chuẩn hoá văn bản tiếng Việt trước tokenizer. `providers.tts.zerotts_onnx.delivery_mode = "stream"` là mặc định: worker dùng `decode_step` và codec cache qua các segment trong một lượt thoại; cold start giải mã theo nhóm 4, 8, 16 frames. Chế độ `"file"` vẫn có thể chọn: worker sinh đủ codec frames của từng Speech Segment, giải mã bằng `decode_full`, ghi WAV float mono 48 kHz vào file tạm, rồi đọc file theo khối. Chỉ sau khi WAV hoàn chỉnh mới đưa PCM vào `SpeechOutput` để đổi mẫu, Opus encode và pace qua WebSocket. File tạm được xoá khi hoàn tất, lỗi hoặc huỷ.

## 2. Flow

```mermaid
flowchart LR
    SEG[Text segment] --> SO[SpeechOutput]
    SO --> TTS[TTS Provider]
    TTS --> PCM[PCM từ file tạm hoặc codec stream]
    PCM --> RS[Normalize/Resample]
    RS --> ENC[Opus encoder 60 ms]
    ENC --> Q[bounded audio queue]
    Q --> P[AudioPacer]
    P --> EV[Started/Sentence/Packet/Drained event]
    EV --> A[SessionActor]
    A --> WS[WS Writer]
```

## 3. TTS state messages

Khi đã có đủ initial prebuffer, hoặc synthesis hoàn tất với câu ngắn dưới ngưỡng, actor tạo:

```text
tts:start
```

Mỗi segment có thể tạo:

```text
tts:sentence_start + text
```

Chỉ sau `FinishInput` và event `Drained`:

```text
tts:stop
```

## 4. Output audio

Output sample rate là config riêng, ví dụ 24 kHz mono. Không dùng input microphone rate làm output rate một cách mặc định.

Opus encoder đóng `DownlinkPcmFrame` đúng 60 ms (1.440 samples ở 24 kHz). Downlink V1 cố định VoIP, 32 kbps, VBR và constrained VBR bật, DTX/FEC tắt, packet-loss percent 0, complexity 10; đây là implementation constants, chưa phải config provider.

`DownlinkOpusEncoder` luôn dùng private `DOWNLINK_ENCODE_BUFFER_BYTES = 4.000`; đó không phải WebSocket cap hay `MAX_UPLINK_OPUS_PACKET_BYTES` dù hiện cùng giá trị. `encode` lỗi, trả zero-byte packet hoặc tạo packet lớn hơn `websocket.max_frame_bytes` là internal delivery failure. Nó không được xử lý như uplink local frame fault và không được đóng WebSocket 1009; Phase 4 map lỗi thành `SpeechOutputEvent::Failed`, drop audio còn lại và chỉ gửi `tts:stop` nếu `tts:start` đã được gửi.

## 5. AudioPacer

Pacer giữ audio đầu turn cho đến khi các segment ngắn đã tổng hợp xong, hoặc hàng đợi của segment dài đạt ngưỡng 32 Opus packet. Sau `tts:start`, 5 packet đầu được gửi ngay; từ packet thứ 6, deadline tính từ thời điểm gói 1 được release: gói 6 ở mốc +60 ms, gói 7 ở +120 ms. Một tick trễ không cộng dồn vào các deadline sau. Ngưỡng ban đầu 32 packet giới hạn thời gian chờ trước phát cho segment dài và tránh kẹt worker khi hàng đợi đầy. `Drained` và `tts:stop` chờ hết thời lượng playback danh nghĩa của các gói đã gửi, kể cả những gói trong burst đầu. Nếu TTS vẫn tạo PCM chậm hơn playback sau ngưỡng này, client có thể thiếu audio; log `TTS Opus buffer` ghi độ sâu hàng đợi khi encode, `Downlink Opus packet ready for WebSocket` ghi trạng thái khi release, còn `WebSocket audio sent` ghi khoảng cách gửi thực tế khi bật mức log `debug`.

```text
prebuffer N frames -> send nhanh
sau đó -> ~frame_ms giữa các packet
```

Giới hạn dispatch/poll PCM khi hàng đợi Opus đạt 32 packet. Segment N+1 có thể bắt đầu trước khi hàng đợi N cạn, miễn chỉ một worker synthesis đang active.

## 6. Segment lifecycle và backpressure

Mỗi segment có `ordinal` liên tục tăng dần. Mỗi generation có tối đa một synthesis active; segment N+1 chỉ dispatch sau `SegmentFinished(N)` và pending queue bounded. `SegmentFinished` chỉ là TTS inference terminal, không phải playback terminal: N+1 có thể bắt đầu inference khi AudioPacer vẫn pace audio N, miễn queue bounded và ordinal playback không reorder. Actor gửi `FinishInput` khi LLM đã flush segment cuối. `SpeechOutput` chỉ phát `Drained` khi đã nhận `FinishInput`, pending rỗng, không active synthesis và packet cuối đã qua pacer; `SegmentFinished` không đủ để gửi `tts:stop`.

TTS producer bị chặn khi bounded queue đầy. Không tạo unbounded audio queue.

Không lấy được worker slot hoặc command queue đầy làm fail-fast current generation; không chờ vô hạn, drop hay skip segment. `tts.timeout_ms` bắt đầu khi worker accept segment và chỉ kết thúc khi `SegmentFinished`, `Failed` hoặc cancelled acknowledgement, không reset theo PCM chunk. Timeout fail generation, request cleanup; hết cleanup grace thì quarantine worker. Không retry logical TTS operation.

`speech_output.pending_segments` (default 8) là hard bound riêng cho text segment chờ synthesis. Khi queue đầy, actor tạm dừng nhận thêm LLM delta cho tới khi TTS lấy bớt segment; nó giữ tối đa một delta đang xử lý dở và không drop hoặc overwrite text. Buffer câu chưa kết thúc vượt ngưỡng khẩn cấp vẫn fail `speech_output_backpressure`. `limits.tts_concurrency` phải bằng `workers.tts.max_workers`; only application admission semaphore cấp permit trước lease native worker.

Trước bind, ZeroTtsFactory chạy deterministic warmup dùng pinned non-user text/voice `maichi`, kiểm tra tokenizer, latent, graph, codec/external data và PCM mono 48 kHz finite/non-empty. Warmup không đi qua SessionActor, SpeechOutput, Opus hay WebSocket, và mutable operation state bị drop/reset trước traffic.

## 7. Cancellation

Khi abort:

1. cancel TTS provider stream.
2. cancel producer và đánh dấu generation cũ stale; không clear shared channel.
3. pacer kiểm tra generation trước send.
4. actor cập nhật `GenerationGate` rồi gửi `tts:stop` session control.

Nếu chưa từng `Started`, failure không gửi `tts:start` hay `tts:stop`. Nếu đã `Started`, actor invalidate generation trước, cancel pipeline, drop audio queued/stale rồi gửi đúng một `tts:stop`; `tts_started`/`tts_stopped` bảo đảm mỗi control tối đa một lần per generation. Không binary audio nào của generation được gửi sau stop.

V1 không automatic retry logical TTS operation sau timeout/lỗi.

## 8. Test contract

- input text -> audio chunks theo đúng order.
- `tts:start` chỉ đến sau first valid AudioPacket và trước packet đó trên wire.
- non-empty input + provider completion + zero valid AudioPacket -> `Failed(tts_empty_audio)`; không `tts:start`/`tts:stop` nếu start chưa gửi.
- resample output đúng target rate.
- Opus packets decode lại thành audio hợp lệ.
- pacer không burst toàn bộ sau prebuffer.
- bounded queue tạo backpressure.
- abort không cho packet generation cũ đi tiếp.
- `tts:stop` sau audio cuối, không trước.
- nhiều segment hoàn tất không theo thời điểm synthesize -> chỉ một `Drained` sau `FinishInput`.
