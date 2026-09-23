# Hướng dẫn cập nhật Long-Lived WebSocket, Turn Isolation và TTS Queue Cleanup

> Repo áp dụng: `hailp-vn38/ai-agent-voice`  
> Repo tham khảo: `xinnan-tech/xiaozhi-esp32-server`  
> Mục tiêu: giữ một WebSocket sống lâu qua nhiều hội thoại liên tiếp, cô lập tuyệt đối từng conversational turn, không để LLM/TTS/audio stale của turn cũ lọt sang turn mới.
> Quyết định kiến trúc: [ADR-0046](adr/0046-long-lived-ws-turn-identity-and-writer-terminal-outcome.md).

---

## 1. Mục tiêu thay đổi

Server hiện đã có nền tảng tốt cho voice session sống lâu:

- mỗi WebSocket có một `SessionActor` duy nhất;
- TTS pipeline dùng bounded queue;
- `SpeechOutput` sở hữu sentence segmentation, synthesis ordering, resample, Opus encode và pacing;
- outbound tách `urgent`, `control`, `audio`;
- `GenerationGate` chặn audio/control stale đã nằm trong outbound queue;
- `CancellationToken` và worker cancel dừng producer cũ;
- `SpeechOutput::cancel()` reset queue/state TTS;
- `tts:stop` bình thường chỉ gửi sau `SpeechOutputEvent::Drained`.

Tuy nhiên cần hoàn thiện một boundary quan trọng:

> Mỗi **Conversational Turn** phải có identity riêng, tăng đơn điệu, không reuse identity của turn trước dù turn trước kết thúc bình thường.

Nhiều turn bình thường được phép cùng `generation` (cancellation epoch), nhưng không được dùng lại `TurnId` hoặc LLM operation identity. ASR stream và VAD capture cycle có identity riêng trước khi có TurnId.

---

## 2. Vấn đề hiện tại

### 2.1. WebSocket và Session đang đúng scope

Một kết nối có lifetime dạng:

```text
WebSocket Session S1
│
├── Turn A
├── Turn B
├── Turn C
├── Turn D
└── ...
```

WebSocket **không nên reconnect sau mỗi câu**.

`SessionActor` vẫn sống xuyên suốt toàn bộ các turn.

### 2.2. Identity của turn hiện chưa đủ mạnh

Hiện `SessionActor` có:

```rust
pub struct SessionActor {
    // ...
    generation: u64,
    turn: Option<TurnContext>,
    // ...
}
```

và worker identity được tạo theo dạng tương tự:

```rust
WorkerIdentity::new(
    self.session_id.clone(),
    self.generation,
    self.generation,
)
```

`generation` tăng trong các đường như:

- abort;
- acoustic barge-in;
- thay listening lifecycle;
- fail closed.

Nhưng một turn hoàn tất bình thường qua:

```text
ASR -> LLM -> TTS -> Drained -> complete_recognition()
```

có thể không tạo identity mới ngay cho turn tiếp theo.

Nếu WebSocket giữ lâu, có thể xuất hiện:

```text
Session S1
Generation 12

Turn A -> ASR/LLM/TTS gen=12 -> Drained
Turn B -> ASR/LLM/TTS gen=12 -> Drained
Turn C -> ASR/LLM/TTS gen=12
```

Việc dùng lại `WorkerIdentity` giữa các LLM operation làm yếu stale-event isolation; bản thân việc nhiều turn cùng generation là hợp lệ.

### 2.3. Rủi ro chính

Nếu callback hoặc worker event của Turn A về rất trễ trong lúc Turn B đang chạy, identity có thể không đủ để chứng minh event đó là stale.

Đặc biệt cần tránh các trường hợp:

```text
A late LLM delta  -> lọt vào SpeechOutput của B
A late TTS event  -> mutate state TTS của B
A late ASR final  -> bắt đầu LLM sai turn
A queued audio    -> phát sau khi B đã bắt đầu
```

Hiện `GenerationGate` xử lý abort; cần thêm **TurnId unique cho mỗi lượt** và operation identity phù hợp tại các ranh giới async. Generation tiếp tục làm cancellation/invalidation epoch.

---

## 3. Nguyên tắc thiết kế mới

### 3.1. Tách Session identity và Turn identity

Các identity có scope khác nhau:

```text
VoiceSessionId
├── GenerationId         cancellation/invalidation epoch
├── ASR Stream Identity   recognition stream trước/sau utterance boundary
├── VadCaptureCycleId     VAD semantic capture cycle
└── TurnId                conversational turn sau Active Turn admission
```

Khuyến nghị:

```rust
pub struct TurnId(u64);
```

và trong actor:

```rust
pub struct SessionActor {
    // session lifetime
    session_id: String,

    // turn lifetime
    next_turn_id: u64,
    delivery_turn: Option<DeliveryTurn>,
    barrier_turn: Option<HistoryBarrierTurn>,

    // cancellation/output invalidation epoch; có thể trải qua nhiều normal turn
    generation: u64,

    // ...
}
```

Không thay `generation` bằng TurnId hoặc ghép TurnId với VAD cycle/ASR stream identity.

### 3.2. `TurnContext` sở hữu cancellation riêng

```rust
struct TurnContext {
    turn_id: TurnId,
    generation: u64,
    cancellation: CancellationToken,
    permit: ActiveTurnPermit,
}
```

Mỗi accepted user utterance phải tạo `CancellationToken` mới.

Token không được reuse.

`ActiveTurnPermit` phải là token/RAII thuộc đúng một TurnContext, được release đúng một lần khi turn terminal hoặc Voice Session teardown. Một cờ `has_active_turn_permit: bool` cấp actor không thể biểu diễn A và B cùng giữ permit.

Trong một Voice Session, state được bounded ở **tối đa một Delivery Turn A và một History-Barrier Turn B**. B vẫn là Active Turn thật, có permit riêng; đây không phải queue turn miễn capacity.

### 3.3. Không coi `listen:start` là turn mới

`listen:start` chỉ là capture/listening control.

Trong Processing/Speaking:

```text
listen:start
    -> arm/reset capture theo policy
    -> KHÔNG tự động tạo TurnId mới
    -> KHÔNG tự động cancel turn hiện tại
```

Turn mới chỉ được cấp khi có user utterance thực sự được chấp nhận.

### 3.4. Async result phải giữ đúng identity theo lifecycle

Các event thuộc Conversational Turn sau Active Turn admission phải correlate được về:

```text
session_id
turn_id
generation hoặc cancellation epoch nếu cần
operation id
```

ASR event trước boundary chỉ mang ASR stream identity; `ASR Final` được liên kết với TurnId qua stream hiện hành. Không dựa riêng vào phase hiện tại để xác định stale event.

---

## 4. Khi nào tạo TurnId mới

**Cấp đúng một TurnId sau utterance terminal boundary, sau khi `ActiveTurnLimiter::try_acquire()` thành công và trước ASR finalization.** Capture bị từ chối capacity không nhận TurnId. Không cấp TurnId khi `listen:start` hoặc `SpeechStart`. Nếu Voice Session đã có History-Barrier Turn B, utterance C qua terminal boundary bị từ chối trước permit acquisition: cancel/revoke ASR C, không STT/TurnId/history/LLM/TTS; không xếp user-turn queue.

### Auto / Realtime

```text
VAD Capture Cycle V8 → SpeechStart → ASR Stream C18 → PCM...
SpeechEnd (utterance terminal boundary)
  → Active Turn permit accepted → allocate TurnId T43
  → ASR Finish(C18) → ASR Final(C18) associated with T43
  → LLM(T43) → TTS(T43)
```

### Manual

```text
listen:start → ASR Stream C17 → PCM...
listen:stop (utterance terminal boundary)
  → Active Turn permit accepted → allocate TurnId T42
  → ASR Finish(C17) → ASR Final(C17) associated with T42
  → LLM(T42) → TTS(T42)
```

### Typed Detect

```text
validated listen:detect → Active Turn permit accepted
  → allocate TurnId → commit_user_text → LLM/TTS
```

Typed Detect không có ASR capture. ASR trước terminal boundary dùng ASR stream identity; VAD cycle luôn độc lập với TurnId.

---

## 5. Helper API nên thêm

### 5.1. Allocate turn

```rust
fn allocate_turn_id(&mut self) -> Result<TurnId, SessionError> {
    let id = self.next_turn_id;
    self.next_turn_id = id.checked_add(1).ok_or(SessionError::TurnIdExhausted)?;
    Ok(TurnId(id))
}
```

Khởi tạo:

```rust
next_turn_id: 1,
```

Không reset `next_turn_id` trong lifetime của WebSocket. Overflow là lỗi invariant: fail closed, không wrap hoặc reuse ID.

### 5.2. Begin turn

```rust
fn begin_turn(&mut self, permit: ActiveTurnPermit) -> Result<TurnContext, SessionError> {
    let turn_id = self.allocate_turn_id()?;
    Ok(TurnContext {
        turn_id,
        generation: self.generation,
        cancellation: CancellationToken::new(),
        permit,
    })
}
```

Thứ tự call site: `try_acquire()` → tạo permit thuộc turn → allocate TurnId → associate ASR stream với TurnId → ASR Finish. Nếu capacity denied, dùng đường hiện tại: cancel ASR, giữ cleanup obligation cho tới acknowledgement, không cấp TurnId hay tạo side effect hội thoại. Nếu TurnId overflow sau khi đã lấy permit, fail closed và release permit qua ownership.

### 5.3. Current turn check

```rust
fn is_current_turn(&self, turn_id: TurnId, generation: u64) -> bool {
    self.delivery_turn.as_ref().is_some_and(|delivery| {
        delivery.context.turn_id == turn_id
            && delivery.context.generation == generation
            && generation == self.generation
    })
}
```

LLM và speech delivery callbacks đi qua check này. ASR callbacks được route bằng ASR stream identity; sau terminal boundary chỉ stream đã liên kết với current TurnId mới được tạo accepted user text. Cleanup acknowledgement vẫn xử lý theo lease dù semantic event đã stale.

---

## 6. WorkerIdentity cần thay đổi

### Hiện trạng không nên tiếp tục

Không nên tiếp tục dùng:

```rust
WorkerIdentity::new(
    session_id,
    generation,
    generation,
)
```

nếu tham số cuối đóng vai trò operation identity.

### Contract đề xuất

`WorkerIdentity` hiện gồm `(session, generation, stream)`. Thành phần `stream` phải unique cho **mỗi worker operation có thể chồng lấn hoặc để lại event muộn** trong cùng Voice Session; không dùng lại `(generation, generation)` cho LLM của hai normal turn.

```rust
// LLM: một operation cho mỗi turn trong contract hiện tại.
WorkerIdentity::new(session_id, generation, turn_id.get())

// ASR: identity đã có trước TurnId.
WorkerIdentity::new(session_id, generation, asr_stream_id.get())
```

Nếu một turn có nhiều LLM invocation, `operation_id` phải unique riêng và được map về TurnId; không reuse cùng identity chỉ vì cùng turn. VAD worker lease và `VadCaptureCycleId` vẫn là hai identity khác nhau; VAD cycle không đổi thành TurnId.

---

## 7. TTS delivery phải correlate với TurnId

Khuyến nghị contract:

```rust
pub struct TtsRequest {
    pub session_id: String,
    pub turn_id: TurnId,
    pub generation: u64,
    pub segment_ordinal: u32,
    pub text: String,
}
```

TTS worker event:

```rust
pub enum TtsWorkerEvent {
    Pcm {
        turn_id: TurnId,
        segment_ordinal: u32,
        pcm: PcmF32Mono,
    },
    Finished {
        turn_id: TurnId,
        segment_ordinal: u32,
    },
    Cancelled {
        turn_id: TurnId,
    },
    Failed {
        turn_id: TurnId,
    },
    TimedOut {
        turn_id: TurnId,
    },
}
```

Đây là contract logic, không bắt buộc đổi ngay tất cả enum runtime. Runtime hiện route TTS event theo `TtsLease`; `SpeechOutput` phải giữ ánh xạ lease/stream/segment tới active TurnId và bỏ event không thuộc lượt đó. Worker cleanup/quarantine tiếp tục do runtime sở hữu; late cleanup acknowledgement không được bị bỏ chỉ vì TurnId đã terminal.

---

## 8. SpeechOutput phải là turn-scoped state machine

Hiện `SpeechOutput::cancel()` reset khá đầy đủ và nên giữ.

Contract cần ghi rõ:

```text
Một SpeechOutput instance logic chỉ phục vụ một active turn tại một thời điểm.
```

State cần reset khi turn terminal:

```rust
self.pending.clear();
self.downlink_tail.clear();
self.downlink_resampler = DownlinkResampler::new_48k_to_24k();
self.first_pcm_chunk = true;
self.packets.clear();
self.packets_sent = 0;
self.finish_input = false;
self.started = false;
self.playback_origin = None;
self.playback_end_deadline = None;
self.json_filter.reset();
self.segmenter.reset();
```

Ngoài ra nên thêm debug assertion/state ownership:

```rust
active_turn_id: Option<u64>,
```

Ví dụ:

```rust
pub struct SpeechOutput {
    active_turn_id: Option<u64>,
    // ...
}
```

Khi begin:

```rust
self.active_turn_id = Some(turn_id);
```

Khi Drained/cancel:

```rust
self.active_turn_id = None;
```

Nếu nhận delta khác TurnId:

```rust
return Err(SpeechOutputError::StaleTurn);
```

---

## 9. Pending TTS segment queue

Giữ mô hình hiện tại:

```text
LLM stream
   ↓
SentenceSegmenter
   ↓
SpeechOutput.pending
   ↓
ONE active TTS synthesis
```

Không synth nhiều sentence song song trong cùng turn nếu provider/codec có state hoặc ordering requirement.

### FIFO bắt buộc

```text
Segment 0
Segment 1
Segment 2
```

phải synthesis đúng thứ tự.

### Cho phép overlap

Được phép:

```text
playback Segment N
       +
synthesis Segment N+1
```

miễn là audio packet vẫn phát đúng order.

---

## 10. Không dùng `clear()` làm correctness mechanism

Repo tham khảo Xiaozhi có cơ chế:

```text
tts_text_queue
tts_audio_queue
sentence_id
client_abort
clear_queues()
```

Rust không nên copy trực tiếp kiểu:

```rust
audio_queue.clear();
control_queue.clear();
```

làm lớp bảo vệ chính.

Lý do:

```text
Thread A: clear queue
Thread B: producer push stale packet ngay sau clear
```

Kết quả stale packet vẫn có thể lọt.

### Cơ chế correctness bắt buộc

```text
CancellationToken
+
TurnId / GenerationGate
+
bounded queue
+
stale check tại consumer
```

Physical queue cleanup chỉ là optimization nếu implement được an toàn.

---

## 11. Giữ GenerationGate và thêm writer-local playback sequencer

Hiện outbound packet có dạng:

```rust
OutboundMessage::Binary {
    generation,
    packet,
}
```

Khuyến nghị:

```rust
OutboundMessage::Binary {
    turn_id,
    generation,
    packet,
}
```

Turn-scoped text:

```rust
OutboundMessage::TurnText {
    turn_id,
    generation,
    text,
}
```

`GenerationGate` tiếp tục là linearization point thread-safe cho cancellation: gate lọc audio và nonterminal turn-scoped JSON của generation đã bị invalidated. **Normal/abort stop không đi qua generic stale gate**; writer phân xử chúng bằng terminal state của TurnId. Không biến gate thành `current_turn_id` gate, vì stop A có thể còn đang chờ khi B đã chuẩn bị.

Writer là một Tokio task duy nhất và sở hữu toàn bộ playback lifecycle. Actor gửi semantic command; writer là nơi duy nhất tạo wire `tts:start`/`tts:stop`:

```rust
enum WriterCommand {
    BeginTurn { generation: GenerationId, turn_id: TurnId },
    Audio { generation: GenerationId, turn_id: TurnId, packet: Vec<u8> },
    FinishTurn { turn_id: TurnId },
    AbortTurn { turn_id: TurnId },
}

enum WriterTurnOutcome {
    Normal,
    Aborted { start_was_sent: bool, stop_was_sent: bool },
}

enum WriterEvent {
    TurnClosed { turn_id: TurnId, outcome: WriterTurnOutcome },
    Failed { turn_id: Option<TurnId> },
}
```

Writer giữ state `NotStarted`, `Started`, `ClosedNormal` hoặc `ClosedAbort` cho lượt đang xử lý. `NotStarted → Abort` không gửi stop; `Started → Finish/Abort` gửi đúng một stop nếu socket còn dùng được. `ClosedNormal → Abort` và `ClosedAbort → Finish` là no-op. Chỉ writer biết `tts:start` đã thực sự được gửi hay chưa; `SpeechOutputEvent::Started` chỉ có nghĩa output đã sẵn sàng/yêu cầu phát. Actor không quyết định gửi stop từ `tts_started`.

Writer phải giữ thứ tự wire `tts:start(A) → audio(A)* → tts:stop(A) → tts:start(B) → audio(B)*`. Hai `AtomicU64` độc lập không thể biểu diễn an toàn playback state này.

**Writer terminal serialization phân xử race:** nếu writer xử lý `AbortTurn(A)` trước normal stop, normal stop đang chờ bị supersede; writer drop audio A và gửi stop chỉ khi start A đã gửi. Nếu writer đã bắt đầu `sender.send(normal stop).await`, lệnh abort chưa được quan sát cho đến khi send trả về: send thành công đóng A với outcome `Normal`, send lỗi làm delivery thất bại. Abort A tới sau `ClosedNormal` không gửi stop lần hai. Không lấy wall-clock lúc actor nhận abort làm mốc quyết định outcome.

### Quan trọng

Gate phải được cập nhật **trước khi cancel producer**.

Abort order:

```text
1. invalidate generation của Turn N tại gate
2. clear actor-local pending_audio
3. cancel SpeechOutput
4. cancel TTS worker
5. cancel LLM
6. cancel ASR
7. gửi `AbortTurn(N)` qua urgent lane; writer gửi stop có điều kiện theo `start_sent`, rồi đóng A trước playback B
8. chỉ release ActiveTurnPermit của A sau `WriterEvent::TurnClosed(A, outcome)` hoặc session teardown
```

---

## 12. Normal completion flow

Contract cần cố định:

```text
LLM Finished
↓
SpeechOutput.finish_input()
↓
pending segment synthesis hết
↓
active TTS worker = none
↓
resampler tail flush
↓
Opus packet cuối
↓
AudioPacer hoàn thành
↓
playback_end_deadline đạt
↓
SpeechOutputEvent::Drained
↓
Actor move generated_response vào PendingDelivery(TurnId)
↓
Actor gửi WriterCommand::FinishTurn(TurnId)
↓
Writer gửi hết audio rồi send normal stop thành công
↓
WriterEvent::TurnClosed(TurnId, Normal)
↓
Actor commit Delivered Assistant Response từ PendingDelivery
↓
release A permit; clear delivery_turn/PendingDelivery
↓
open B history barrier (nếu có), rồi re-arm VAD/listening theo mode
```

`SpeechOutput::Drained` chỉ chứng minh producer/pacer phía server đã xong, chưa phải Delivered. Actor giữ `PendingDelivery { turn_id, assistant_text }` cho tới terminal writer outcome; không clear text khi nhận abort trong lúc outcome còn chưa biết. Outcome `Normal` commit, outcome `Aborted` discard. Normal finish queue admission failure hoặc WebSocket send failure không được commit assistant history; nếu admission thất bại trong khi socket còn sống thì fail closed. `sender.send()` thành công không chứng minh client đã phát xong; semantic client playback completion cần protocol acknowledgement riêng. Không gửi `tts:stop` ngay khi model TTS synth xong.

**History barrier:** Capture và ASR của B có thể overlap writer-finalization của A. B qua SpeechEnd/listen:stop chỉ được admission khi global limiter còn capacity và session chưa có History-Barrier Turn khác; B giữ permit riêng, nhận TurnId trước ASR Finish. Nếu global capacity là 1 và A còn giữ permit, B bị từ chối theo đường capacity hiện tại. Khi capacity cho phép, `ASR Final(B)` có thể đến trước writer outcome A; actor cho ASR worker B terminal/cleanup bình thường, giữ final text B tối đa **4.096 Unicode scalar** (cùng giới hạn typed Detect), không truncate. Text vượt bound là controlled ASR/turn failure: release permit B, không STT/history/LLM. Chỉ có một B pending nên bộ nhớ per-session vẫn bounded.

Actor không commit accepted user text B, không dựng prompt và không start LLM B cho tới khi nhận terminal writer outcome của A. Khi đó history mới có thứ tự xác định: `User A → Assistant A → User B` nếu A `Normal`, hoặc `User A → User B` nếu A `Aborted`. Xử lý outcome A trong **một actor mailbox turn**: resolve history A, release permit A, remove A context, mở barrier B, rồi kích hoạt B nếu final text đã sẵn sàng. Nếu B vẫn đang ASR Finalizing, outcome A chỉ mở barrier; ASR Final(B) về sau sẽ commit B và start LLM ngay. B Final rỗng/ASR lỗi terminal và release permit B ngay, không cần đợi A vì không có nội dung làm đổi history.

Nếu B bị abort khi chờ, discard final text, cancel B, release permit B và remove B context. Writer outcome A về sau chỉ resolve A, không hồi sinh B. Nếu writer A lỗi, Voice Session teardown: release cả hai permit, discard B text, tiếp tục ASR cleanup obligation theo worker contract; không start LLM B.

---

## 13. Abort flow

V1 `abort` chỉ mang `session_id`, không chọn TurnId. Nó revoke **mọi conversational/capture work chưa terminal** của Voice Session tại lúc actor accept command. Với A đang chờ writer outcome và B đang chờ history barrier, actor xử lý một inbound abort như sau:

```text
1. invalidate current generation tại GenerationGate
2. take/remove History-Barrier Turn B; revoke semantic ASR ownership,
   cancel ASR nếu còn Finalizing, discard FinalReady text, release permit B
3. nếu Delivery Turn A còn trong actor, gửi urgent AbortTurn(A)
4. cancel/revoke capture và ASR khác theo lifecycle hiện hành
5. checked_add generation đúng một lần; overflow fail closed
6. reset/re-arm Auto/Realtime theo policy hiện hành
```

Nếu A thuộc generation cũ do acoustic barge-in, generation đó đã bị invalidate tại interruption trước; inbound abort hiện tại invalidate current generation của B/capture, không tăng epoch riêng cho từng turn. `FinishTurn(A)` và `AbortTurn(A)` là writer lifecycle commands, không bị generic GenerationGate lọc. B phải bị remove ngay; late ASR Final(B) bị từ chối theo identity/ownership, và writer outcome A về sau không thể tái kích hoạt B.

Actor **không tự đánh dấu A Aborted và không release permit A khi gửi AbortTurn**. Writer phân xử A theo Q4; permit A chỉ release khi `TurnClosed(A, Normal|Aborted)` hoặc Voice Session teardown. Nếu normal stop A đã thành công trước khi writer xử lý abort, A vẫn Normal và assistant A được commit khi event tới; B vẫn Aborted, nên history có `User A, Assistant A` nhưng không có User B. Nếu A không còn trong delivery slot, không gửi AbortTurn(A). Abort lặp lại phải idempotent: không double-release hoặc double-stop. Tên `abort_current_turn()` nên đổi thành `abort_active_interaction()` hoặc `abort_session_work()` vì scope giờ rộng hơn một turn.

### Trước khi writer gửi `tts:start`

```text
abort
↓
invalidate generation tại gate
↓
cancel LLM/TTS/ASR
↓
reset SpeechOutput
↓
urgent WriterCommand::AbortTurn(TurnId)
↓
writer drop queued start/audio/normal finish; không gửi stop
↓
TurnClosed(TurnId, Aborted { start_was_sent: false, stop_was_sent: false })
```

### Sau khi writer đã gửi `tts:start`

```text
abort
↓
invalidate generation của Turn N tại gate
↓
drop actor pending_audio
↓
cancel TTS producer
↓
cancel LLM
↓
cancel ASR
↓
urgent WriterCommand::AbortTurn(TurnId)
↓
writer gửi đúng một stop, kể cả khi chưa gửi audio
↓
TurnClosed(TurnId, Aborted { start_was_sent: true, stop_was_sent: true })
```

Nếu normal stop đã gửi thành công trước khi writer xử lý abort, outcome A là `Normal`; AbortTurn(A) là no-op, không gửi stop lần hai. Nếu normal stop send đang await, send thành công cho `Normal` thắng; send lỗi là connection failure và không Delivered. Duplicate abort hoặc delayed normal finish sau abort không được tạo stop thứ hai.

### Không commit assistant history

Writer outcome `Aborted` hoặc connection failure sau partial playback:

```text
KHÔNG commit full assistant response vào Delivered History
```

trừ khi project sau này định nghĩa riêng partial-delivery history contract.

---

## 14. Audio đã gửi qua WebSocket không thể thu hồi

Server chỉ drop được packet chưa gửi.

Ví dụ:

```text
server                         client

packet 1 -------------------->
packet 2 -------------------->
packet 3 -------------------->

          ABORT

packet 4 -> dropped
packet 5 -> dropped
```

Client có thể vẫn đang buffer packet 2/3.

Do đó client contract phải yêu cầu:

```text
on tts:stop caused by abort:
  stop decoder/playback
  clear local playback queue
  discard queued Opus belonging to old turn
```

Nếu protocol chưa có TurnId trên wire binary, `tts:stop` vẫn phải là immediate playback flush boundary.

---

## 15. Giữ urgent lane riêng

Không gộp `urgent_tx` vào `control_tx`.

Writer priority nên giữ:

```text
shutdown
>
urgent control
>
normal stop after audio drain
>
normal control
>
audio
```

`AbortTurn` phải đi urgent lane; writer tạo stop có điều kiện từ state đã gửi start.

Normal `tts:stop` vẫn phải đợi audio queue/drain đúng order.

---

## 16. Queue sizing

Hiện `SpeechOutput` có internal packet buffering.

Cần theo dõi tổng buffer:

```text
SpeechOutput packets
+
SessionActor pending_audio
+
outbound_audio_queue
+
WebSocket implementation buffer
+
client playback buffer
```

Không chỉ nhìn một queue riêng lẻ.

### Mục tiêu

Giảm lượng audio đã gửi trước quá xa so với playback realtime để barge-in phản hồi nhanh.

Không tăng queue lớn chỉ để tránh backpressure.

Backpressure phải propagate ngược:

```text
WS slow
↓
audio_tx full
↓
pending_audio giữ 1 packet
↓
SpeechOutput ngừng drain
↓
internal packet high-water mark
↓
TTS worker event route backpressure
↓
LLM/Speech segment admission chậm lại
```

Đây là behavior mong muốn.

---

## 17. File cần sửa

### 17.1. `crates/voice-agent-server/src/session/actor/mod.rs`

Thêm:

```rust
next_turn_id: u64,
```

Thay actor-wide `has_active_turn_permit: bool` bằng permit RAII thuộc từng `TurnContext`. Model rõ hai slot bounded, không dùng `HashMap<TurnId, TurnContext>` cho arbitrary turns:

```rust
struct TurnContext {
    turn_id: TurnId,
    generation: u64,
    cancellation: CancellationToken,
    permit: ActiveTurnPermit,
}

struct SessionActor {
    delivery_turn: Option<DeliveryTurn>,
    barrier_turn: Option<HistoryBarrierTurn>,
    // ...
}

struct HistoryBarrierTurn {
    context: TurnContext,
    asr_state: BarrierAsrState,
    history_barrier_open: bool,
}

enum BarrierAsrState {
    Finalizing { asr_identity: WorkerIdentity },
    FinalReady { text: String },
}
```

Tên code có thể là `PendingTurn`; trong domain docs dùng **History-Barrier Turn** để rõ đây vẫn là Active Turn. `try_activate_pending_turn()` chỉ commit user text/start LLM khi barrier đã mở và FinalReady. B chuyển sang delivery slot; không tạo turn thứ ba khi barrier slot còn chiếm.

Nếu cần:

```rust
next_operation_id: u64,
```

### 17.2. `crates/voice-agent-server/src/session/actor/delivery.rs`

Sửa:

- allocate TurnId ở Active Turn admission; `begin_speech_delivery()` nhận TurnId đã cấp;
- LLM identity phải chứa TurnId unique;
- `on_llm_event()` check đúng current turn;
- `drain_speech_output()` tạo outbound packet với TurnId;
- `cancel_speech_delivery()` invalidate generation trước producer cancellation;
- `Drained` lưu `PendingDelivery` rồi gửi `FinishTurn`; chỉ `WriterEvent::TurnClosed(Normal)` mới commit history, release permit và terminalize turn;
- abort trong khi `PendingDelivery` đang chờ không tự discard assistant text; writer outcome phân xử;
- `ASR Final(B)` phải chờ terminal outcome A trước khi commit user B hoặc start LLM B;
- outcome A được xử lý atomically trong một actor mailbox turn; B đang Finalizing thì chỉ mở barrier, B FinalReady thì kích hoạt;
- session-scoped abort remove B ngay, giữ permit A tới typed writer terminal outcome; stale/duplicate outcome lookup theo TurnId;
- không reuse turn identity cho response tiếp theo.

### 17.3. `crates/voice-agent-server/src/session/actor/listening.rs`

Sửa các điểm tạo conversational turn:

- Auto SpeechEnd sau Active Turn admission;
- Manual listen:stop sau Active Turn admission;
- Detect sau validate và Active Turn admission.

Acoustic barge-in mở ASR mới trước TurnId; chỉ cấp TurnId mới sau SpeechEnd/admission. Giữ mapping từ ASR stream identity sang TurnId sau boundary.

Khi B qua terminal boundary, check barrier slot trước, sau đó acquire permit rồi allocate TurnId trước ASR Finish. Nếu capacity denied hoặc slot đã chiếm, cancel ASR B và giữ physical cleanup obligation; không STT/TurnId/history/LLM/TTS. Final text B phải được bound 4.096 Unicode scalar; empty/failed/oversize terminalizes B và release permit đúng một lần.

`listen:start` trong Processing/Speaking vẫn chỉ arm, không allocate TurnId.

### 17.4. `crates/voice-agent-server/src/session/speech_output/mod.rs`

Khuyến nghị thêm:

```rust
active_turn_id: Option<u64>,
```

và API turn-aware:

```rust
begin_turn(turn_id)
push_delta(turn_id, text)
finish_input(turn_id)
cancel_turn(turn_id)
```

Nếu muốn migration nhẹ, có thể giữ API cũ nhưng `SessionActor` phải đảm bảo SpeechOutput được reset trước khi bind turn mới.

### 17.5. `crates/voice-agent-server/src/session/speech_output/pacing.rs`

Giữ cleanup hiện tại.

Bổ sung invariant:

```text
Drained chỉ thuộc active_turn_id hiện tại và không tự terminalize turn ở actor.
```

Sau Drained:

```rust
active_turn_id = None;
```

### 17.6. `crates/voice-agent-server/src/workers/...`

Tìm tất cả `WorkerIdentity` cho:

- ASR;
- LLM;
- TTS;
- VAD lease/cycle theo identity riêng; không bắt buộc mang TurnId trước utterance boundary.

Đảm bảo operation identity không reuse giữa hai turn.

### 17.7. `crates/voice-agent-server/src/app/websocket.rs`

Giữ GenerationGate để lọc cancellation/stale audio và nonterminal control; thêm writer-local playback sequencer nhận `BeginTurn/Audio/FinishTurn/AbortTurn` và trả `WriterEvent::TurnClosed(TurnId, outcome)`. Writer là owner duy nhất của wire `tts:start`/`tts:stop`; terminal stop không đi qua generic stale gate.

Ví dụ:

```rust
OutboundMessage::Binary {
    turn_id,
    generation,
    packet,
}
```

Writer phải drop stale trước `sender.send()`.

---

## 18. Migration theo từng bước

### Step 1 — Thêm TurnId nhưng chưa xóa generation

Giữ `generation` để không phá cancellation hiện tại.

Thêm:

```rust
next_turn_id
TurnContext.turn_id
```

### Step 2 — Làm WorkerIdentity unique theo operation

ASR trước boundary dùng stream identity. LLM và speech delivery sau boundary phải trace được về:

```text
SessionId + TurnId
```

### Step 3 — Gắn TurnId vào outbound

Audio và turn control phải có turn identity nội bộ.

### Step 4 — Writer-local playback sequencer

Writer giữ thứ tự start/audio/stop qua các turn, phân xử Finish/Abort race và trả typed terminal outcome; GenerationGate vẫn lọc generation bị cancel. Actor giữ PendingDelivery và history barrier cho B tới khi A terminal.

### Step 5 — SpeechOutput ownership

Bảo đảm SpeechOutput không nhận delta/event của turn khác.

### Step 6 — Tests cho long-lived WS

Chỉ khi test nhiều turn pass mới coi migration hoàn tất.

### Step 7 — Giữ hai semantic riêng

`TurnId` là conversational identity; `generation` là cancellation/invalidation epoch. Nhiều normal turn có thể cùng generation. Không đổi một identity thành tên khác của identity kia.

---

## 19. Test bắt buộc

### 19.0. Regression identity trùng giữa hai normal turn

```text
Turn A kết thúc bình thường ở generation G
Turn B bắt đầu ở cùng G
inject late TextDelta/Finished(A) sau khi B có LLM operation
```

Trước migration, `(session, G, G)` có thể trùng identity. Sau migration, A/B có operation identity khác nhau; late event A phải bị từ chối trước khi thay đổi generated response, SpeechOutput hoặc history B.

### 19.1. Nhiều turn bình thường trên một WS

```text
connect once
Turn 1 -> complete
Turn 2 -> complete
Turn 3 -> complete
Turn 4 -> complete
```

Assert:

```text
same session_id
unique monotonic turn_id
không reconnect
mỗi turn đúng một tts:start/stop
không audio overlap sai turn
writer `TurnClosed(Normal)` trước khi actor commit assistant history
```

### 19.2. Late LLM event từ turn trước

```text
Turn A completed
Turn B started
inject late TextDelta(A)
```

Expected:

```text
ignored
không mutate generated_response(B)
không enqueue SpeechOutput(B)
```

### 19.3. Late ASR Final

```text
Turn A cancelled
Turn B active
ASR Final(A) arrives late
```

Expected:

```text
drop
không start LLM(A)
```

### 19.4. Late TTS PCM

```text
Turn A abort
Turn B active
TTS PCM(A) arrives late
```

Expected:

```text
drop/quarantine theo worker contract
không encode vào SpeechOutput(B)
```

### 19.5. Abort khi outbound audio queue đang full

Setup:

```text
Turn A speaking
outbound_audio_queue full
pending_audio = Some(A)
```

Abort.

Expected:

```text
pending_audio = None
GenerationGate invalidated trước producer cancel
urgent AbortTurn admitted
all queued A audio dropped
no binary after stop
exactly one stop nếu writer đã gửi start; không stop nếu start chưa gửi
```

### 19.6. Abort rồi turn mới ngay lập tức

```text
Turn A speaking
abort A
Turn B starts immediately
```

Assert:

```text
stop(A) before first audio(B)
no audio(A) after stop(A)
B uses new TurnId
B LLM context chỉ dựng sau terminal writer outcome A
```

### 19.7. Normal Drained rồi turn mới

```text
A SpeechOutput Drained
A audio còn queue; normal stop A deferred
B input có thể đang được chuẩn bị
writer send final audio A → stop A → TurnClosed(A, Normal)
actor terminal-complete A
writer mới send start/audio B
```

Assert:

```text
SpeechOutput state reset
TTS stream fresh/valid
segment ordinal B starts at 0
packet counter B starts at 0
fade-in applies correctly for B
resampler tail A không leak sang B
stop A không bị current-turn filtering loại bỏ
```

### 19.7a. Normal stop failure

Nếu normal stop không vào được bounded writer path hoặc `sender.send()` thất bại: không commit assistant A, không claim Delivered, và fail closed/teardown Voice Session.

### 19.7b. Writer terminal race và start visibility

Test deterministic ở writer boundary:

1. `FinishTurn(A)` đang deferred, `AbortTurn(A)` được writer xử lý trước send: outcome `Aborted`, không commit.
2. Normal stop send thành công trước abort, nhưng actor chưa nhận event: outcome `Normal`, commit sau event, không gửi stop thứ hai.
3. Abort đến khi normal stop send đang await: send thành công cho `Normal` thắng; send lỗi cho connection failure, không commit.
4. Abort trước khi queued start được gửi: không start, audio hoặc stop; outcome `Aborted`.
5. Abort sau start nhưng trước audio đầu: đúng một start và stop, không audio.
6. Abort sau partial audio: đúng một stop, không audio A sau stop.
7. Duplicate abort hoặc delayed `FinishTurn` sau abort: không gửi stop lần hai.
8. B ready khi A terminal pending: không start/audio B trước terminal outcome A.
9. ASR Final(B) trước writer outcome A: không commit user B hoặc start LLM B cho tới khi history A được resolve.

### 19.7c. History-Barrier Turn và capacity

1. Capacity = 1, A chưa terminal: B terminal boundary bị capacity denied, không TurnId/STT/history/LLM; ASR B cleanup acknowledgement vẫn được xử lý.
2. Capacity ≥ 2, A pending writer và B được admission: hai permit riêng; B có TurnId trước ASR Finish, ASR Final B có thể đến trước outcome A, ASR lease cleanup không chờ writer.
3. B FinalReady trước A: giữ tối đa một transcript ≤4.096 Unicode scalar; outcome A `Normal` commit Assistant A rồi User B/start LLM B trong cùng actor mailbox turn; `Aborted` chỉ commit User B.
4. A terminal khi B còn Finalizing: mở barrier; ASR Final B đến sau thì commit User B/start LLM B.
5. B Final rỗng, ASR lỗi hoặc transcript vượt bound: release permit B, không user commit; không truncate transcript.
6. B bị abort khi chờ barrier: discard text và release permit B; writer outcome A về sau không hồi sinh B.
7. B đã chiếm barrier slot, C qua terminal boundary: reject C trước permit/TurnId, cancel ASR C và giữ cleanup obligation; không tạo hàng đợi lượt.
8. Writer A lỗi và socket teardown: release cả permit A/B, discard B text, không LLM B; physical ASR cleanup vẫn do runtime theo dõi.

### 19.7d. Session-scoped abort khi A+B cùng tồn tại

1. A writer pending, B FinalReady: abort remove B/discard text/release permit B ngay, gửi AbortTurn(A); giữ permit A tới writer outcome.
2. A writer pending, B ASR Finalizing: revoke B semantic ASR ownership, cancel nếu còn có thể, giữ cleanup obligation; late Final(B) bị bỏ, không recreate B.
3. Normal stop A thắng abort race: commit Assistant A khi `TurnClosed(A, Normal)` về, không commit User B, không stop A lần hai.
4. AbortTurn(A) thắng: không Assistant A, không User B; A permit release đúng một lần trên `TurnClosed(A, Aborted)`.
5. Duplicate abort: generation chỉ advance một lần cho mỗi inbound abort được accept, không double-release permit hoặc double-stop; abort idempotent khi không còn work.
6. Abort sau khi writer đóng A nhưng trước actor ACK: A vẫn Normal, B bị abort, actor không suy luận outcome A từ thời điểm nhận abort.
7. Abort khi chỉ còn B: không phát AbortTurn(A); abort khi không có turn/capture vẫn tuân lifecycle reset/re-arm hiện hành.

### 19.8. 100+ turn soak test

Một WebSocket duy nhất:

```text
for turn in 1..=100:
    submit utterance
    await tts:stop
```

Kiểm tra:

- memory không tăng tuyến tính;
- queue quay về baseline sau mỗi turn;
- worker slot được release;
- history bounded theo config;
- không stale audio;
- không reconnect;
- không duplicate `tts:start/stop`;
- TurnId strictly monotonic.

### 19.9. Mixed completion/abort soak

Pattern:

```text
normal
abort during LLM
normal
abort during TTS
normal
barge-in
normal
```

chạy lặp nhiều vòng trên cùng WS.

---

## 20. Telemetry cần thêm

Mọi log turn-scoped nên có:

```text
session_id
turn_id
generation
operation_id
segment_ordinal
```

Ví dụ:

```rust
tracing::info!(
    session_id = %self.session_id,
    turn_id,
    generation = self.generation,
    "TTS delivery started"
);
```

Audio packet:

```rust
tracing::debug!(
    turn_id,
    packet_seq,
    delta_ms,
    "WebSocket audio sent"
);
```

Abort:

```rust
tracing::info!(
    turn_id,
    generation,
    "Conversational turn invalidated"
);
```

Không log PCM/audio raw.

---

## 21. Metrics nên có

```text
voice_turn_started_total
voice_turn_completed_total
voice_turn_aborted_total
voice_turn_failed_total

stale_asr_event_total
stale_llm_event_total
stale_tts_event_total
stale_outbound_audio_dropped_total

speech_output_pending_segments
speech_output_buffered_packets
outbound_audio_queue_depth

active_tts_workers
active_llm_operations
active_asr_streams
```

Các metric stale phải gần 0 trong normal operation nhưng test cancellation bắt buộc chứng minh chúng hoạt động.

---

## 22. Invariants cần ghi thành comment/test

### Invariant A

Một WebSocket có thể chứa nhiều TurnId.

```text
SessionId stable
TurnId monotonic
```

### Invariant B

Tại một SessionActor chỉ có tối đa một conversational turn sở hữu assistant delivery.

Actor có thể đồng thời giữ một Delivery Turn A và một History-Barrier Turn B; cả hai là Active Turn và mỗi lượt sở hữu một permit riêng. Không có B thứ hai trong barrier slot.

### Invariant C

Một stale async event không được mutate state của current turn.

### Invariant D

Sau interruption stop:

```text
no binary packet belonging to interrupted turn may pass WS writer
```

### Invariant E

Normal `tts:stop` chỉ xuất hiện sau final paced audio của turn đó.

### Invariant F

`SpeechOutput::Drained` chỉ đóng producer của đúng một turn; `WriterEvent::TurnClosed(Normal)` sau normal stop send thành công mới terminalizes turn và cho phép commit Delivered Assistant Response.

### Invariant G

`listen:start` không tự động là interruption.

### Invariant H

TTS worker slot chỉ reusable sau cleanup acknowledgement hoặc quarantine policy.

### Invariant I

TurnId cấp đúng một lần sau utterance terminal boundary và Active Turn admission; ASR stream/VAD cycle trước boundary giữ identity riêng. TurnId overflow fail closed.

### Invariant J

Writer không gửi `tts:start(B)` hoặc audio B trước terminal stop A được xử lý thành công. Client playback completion không được suy ra từ server send.

### Invariant K

Writer phân xử Finish/Abort race theo terminal action nó xử lý thành công trước, không theo thời điểm wall-clock actor nhận abort. Một normal stop send đã bắt đầu không bị preempt: success là `Normal`, error là connection failure.

### Invariant L

Mỗi TurnId có tối đa một wire start và một wire stop; stop không tồn tại nếu start chưa gửi. Nếu start đã gửi và connection còn dùng được, terminal outcome cần đúng một stop.

### Invariant M

Terminal writer outcome của A là history/context barrier: ASR B được overlap, nhưng accepted user B và LLM B chờ outcome A.

### Invariant N

Admission B cần global capacity và barrier slot trống; lấy permit trước TurnId, TurnId trước ASR Finish. Capacity/slot denial không tạo TurnId, STT hay history. Permit thuộc từng TurnContext và chỉ release một lần.

### Invariant O

Final text của tối đa một History-Barrier Turn được giữ bounded ở 4.096 Unicode scalar; oversize/empty/failure terminalizes B mà không commit user. Writer outcome A và B activation được xử lý trong cùng một actor mailbox turn.

### Invariant P

V1 abort là session-scoped: revoke mọi conversational/capture work chưa terminal. B bị remove/release ngay; A chỉ terminal theo writer outcome và giữ permit tới outcome hoặc teardown. Outcome A Normal đã linearize vẫn Normal dù actor nhận abort trước ACK; không hồi sinh B.

### Invariant Q

Mỗi inbound abort tạo tối đa một generation epoch transition bằng checked increment; overflow fail closed. Terminal writer lifecycle commands không bị GenerationGate drop.

---

## 23. Không thay đổi các behavior đang đúng

Không thay các điểm sau nếu không có reason riêng:

- WebSocket sống lâu;
- `SessionActor` là mutable owner duy nhất;
- bounded channels;
- `pending_audio` chỉ giữ tối đa một packet ngoài queue;
- urgent lane cho `AbortTurn` và stop do writer tạo;
- normal stop deferred sau audio;
- một active TTS synthesis mỗi turn;
- TTS synthesis có thể overlap playback;
- `SpeechOutput::cancel()` reset resampler/encoder-side delivery state;
- provider worker không gửi trực tiếp WebSocket;
- actor là outbound producer semantic duy nhất; writer là owner duy nhất của wire start/stop;
- history chỉ commit assistant sau `TurnClosed(Normal)`.

---

## 24. So sánh với Xiaozhi để tham khảo

Xiaozhi dùng:

```text
sentence_id
client_abort
tts_text_queue
tts_audio_queue
AudioRateController
```

và drop audio cũ khi:

```text
message.sentence_id != conn.sentence_id
```

Rust nên lấy **ý tưởng identity-per-turn** này nhưng không copy queue architecture Python.

Mapping hợp lý:

```text
Xiaozhi sentence_id
        ↓
Rust TurnId
```

```text
Xiaozhi client_abort
        ↓
Rust CancellationToken + GenerationGate invalidation
```

```text
Xiaozhi queue clear
        ↓
Rust stale filtering + bounded queue + producer cancellation
```

Rust giữ được concurrency safety tốt hơn nếu tiếp tục coi identity filtering là correctness mechanism chính.

---

## 25. Definition of Done

Update chỉ được coi là hoàn tất khi toàn bộ điều kiện sau đạt:

- [ ] Một WebSocket chạy liên tục nhiều turn không reconnect.
- [ ] Mỗi conversational turn có `TurnId` unique monotonic.
- [ ] `listen:start` không tự tạo turn mới khi chưa có utterance.
- [ ] ASR stream trước boundary có identity riêng và được liên kết với TurnId sau admission; LLM/TTS correlate về TurnId.
- [ ] LLM WorkerIdentity không reuse giữa hai normal turn; operation identity không reuse khi event muộn còn có thể đến.
- [ ] SpeechOutput không nhận/commit event stale từ turn cũ.
- [ ] Outbound audio mang turn identity nội bộ.
- [ ] Writer drop stale audio trước `sender.send()`.
- [ ] Abort invalidates gate trước producer cancellation.
- [ ] Writer tự quyết định stop từ `start_sent`: abort trước wire start không gửi stop, abort sau start gửi đúng một stop.
- [ ] Không audio của turn cũ xuất hiện sau abort stop.
- [ ] Normal `tts:stop` chỉ sau `Drained`.
- [ ] `Drained` reset SpeechOutput nhưng chưa đánh dấu Delivered.
- [ ] Writer trả `TurnClosed(TurnId, Normal|Aborted)`; actor chỉ commit assistant trên `Normal` và giữ PendingDelivery tới lúc outcome được phân xử.
- [ ] Normal/abort race được linearize tại writer; normal stop send in-flight thành công cho Normal thắng, lỗi thì không Delivered.
- [ ] ASR B có thể overlap, nhưng commit user B và start LLM B chờ terminal outcome A.
- [ ] Tối đa một Delivery Turn và một History-Barrier Turn trong một Voice Session; cả hai giữ permit riêng khi cùng tồn tại.
- [ ] Capacity/slot denial của B hoặc C không cấp TurnId/STT/history; ASR cleanup vẫn được xử lý.
- [ ] Pending ASR Final B bounded 4.096 Unicode scalar; empty/failed/oversize release permit B, không truncate/commit.
- [ ] Outcome A và activation B xử lý trong một actor mailbox turn; writer failure teardown release cả hai permit.
- [ ] Session-scoped abort remove B ngay, revoke ASR B, không commit User B hoặc hồi sinh B từ ACK A; permit A chỉ release trên terminal writer outcome/teardown.
- [ ] Abort A Normal race giữ Assistant A nếu writer đã đóng Normal, nhưng B vẫn Aborted; duplicate abort không double-stop/release.
- [ ] Mỗi abort advance generation đúng một lần bằng checked increment; terminal writer commands không qua generic stale gate.
- [ ] Normal stop admission/send failure không commit assistant và fail closed/teardown.
- [ ] Writer không cho start/audio B vượt stop A.
- [ ] 100+ turn soak test pass trên một WS.
- [ ] Mixed abort/normal/barge-in soak test pass.
- [ ] Queue depth quay về baseline sau mỗi turn.
- [ ] Không tăng memory tuyến tính theo số turn.
- [ ] `cargo fmt --check` pass.
- [ ] workspace tests pass.
- [ ] Clippy với warnings denied pass.

---

## 26. Thứ tự implementation khuyến nghị

```text
1. Introduce TurnId
2. Allocate TurnId sau terminal boundary và Active Turn admission
3. Put TurnId và RAII ActiveTurnPermit vào TurnContext; thêm bounded delivery/barrier slots
4. Make LLM WorkerIdentity unique per turn; giữ ASR stream identity riêng
5. Add TurnId to LLM/TTS routing
6. Add TurnId to outbound turn messages/audio
7. Giữ GenerationGate; thêm writer-local playback owner và typed terminal outcome
8. Add SpeechOutput active_turn ownership
9. Add regression late LLM event, writer terminal race/start visibility, stop failure, history barrier/capacity, session-scoped abort
10. Add long-lived WS multi-turn test
11. Add 100+ turn soak test
12. Giữ TurnId và generation với semantic riêng
```

Không nên refactor toàn bộ cancellation/generation trong cùng một commit đầu tiên. Trước tiên thêm TurnId song song với architecture hiện tại, chứng minh behavior bằng test, sau đó mới cleanup abstraction.

---

## 27. Kết luận kiến trúc

Mô hình cuối nên là:

```text
Long-lived WebSocket Session
│
├── Turn 1
│   ├── ASR
│   ├── LLM
│   └── TTS -> Drained -> TurnClosed(Normal) -> Delivered
│
├── Turn 2
│   ├── ASR
│   ├── LLM
│   └── TTS -> Abort
│
├── Turn 3
│   ├── ASR
│   ├── LLM
│   └── TTS -> Drained -> TurnClosed(Normal) -> Delivered
│
└── ...
```

Correctness không phụ thuộc vào việc queue đã được `clear()` hay chưa.

Correctness phải đến từ:

```text
unique TurnId
+
CancellationToken
+
GenerationGate + writer-local playback sequencing
+
bounded backpressure
+
stale filtering tại async boundaries
```

Đây là hướng phù hợp nhất cho server Rust giữ WebSocket dài hạn và xử lý nhiều hội thoại liên tiếp an toàn.
