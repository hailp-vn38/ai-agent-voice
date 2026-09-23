# Phase 5 — Interruption và acoustic barge-in correctness

> Mục tiêu: cập nhật `hailp-vn38/ai-agent-voice` để khi AI đang phát TTS, nếu client có AEC và người dùng bắt đầu nói thì server có thể ngắt turn cũ, giữ lại chính audio gây interrupt, tiếp tục ASR cho turn mới, đồng thời bảo đảm không có stale LLM/TTS/audio của turn cũ lọt ra WebSocket.

## 0. Baseline đã kiểm tra

Tài liệu này được đối chiếu tại các revision sau:

- Rust project `hailp-vn38/ai-agent-voice`: `113838d521da3c7968d2badc9b41818e3e3c100f`
- Reference `xinnan-tech/xiaozhi-esp32-server`: `788f5301fdd60cc3a8ef74025bfeece9b82b94ce`

Các file Rust liên quan trực tiếp:

- `docs/06-implementation-plan.md`
- `docs/flows/06-interrupt-cancellation.md`
- `crates/voice-agent-server/src/protocol/client.rs`
- `crates/voice-agent-server/src/session/state.rs`
- `crates/voice-agent-server/src/session/actor/mod.rs`
- `crates/voice-agent-server/src/session/actor/ingress.rs`
- `crates/voice-agent-server/src/session/actor/listening.rs`
- `crates/voice-agent-server/src/session/actor/delivery.rs`
- `crates/voice-agent-server/src/session/actor/lifecycle.rs`
- `crates/voice-agent-server/src/app/websocket.rs`
- `crates/voice-agent-server/src/session/speech_output/mod.rs`

Các file Xiaozhi tham khảo:

- `main/xiaozhi-server/core/handle/receiveAudioHandle.py`
- `main/xiaozhi-server/core/handle/abortHandle.py`
- `main/xiaozhi-server/core/handle/sendAudioHandle.py`
- `main/xiaozhi-server/core/handle/textHandler/listenMessageHandler.py`
- `main/xiaozhi-server/core/providers/asr/base.py`
- `main/xiaozhi-server/core/providers/tts/base.py`
- `main/xiaozhi-server/core/providers/vad/silero.py`
- `main/xiaozhi-server/core/connection.py`
- `main/xiaozhi-server/core/handle/helloHandle.py`

## 0.1. Quyết định đã chốt

Các quyết định dưới đây là contract authoritative của Phase 5; chúng thay thế mọi đề xuất mâu thuẫn ở các phần sau của guide cũ.

1. `listen:start` chỉ arm/reset **VAD Capture Cycle** và set/replace Listening Mode. Nó không cancel delivery, không invalidate GenerationId và không tăng generation khi assistant đang `Speaking`. Chỉ explicit `abort` hoặc Acoustic Barge-in `SpeechStart` được phép interrupt assistant.
2. Cả `Auto` và `Realtime` dùng chung Silero/VAD, retention và ASR. Auto chỉ watch khi cycle đã arm; Realtime giữ cycle armed xuyên `Processing`/`Speaking`. Manual không acoustic barge-in.
3. `features.aec=true` là Echo-safe Client Assertion, không phải server-side AEC. Predicate bắt buộc là `barge_in.enabled && barge_in.trust_client_aec_feature && client_features.aec && mode in {Auto, Realtime}`. Hai config mặc định `false`.
4. Sau `tts:start`, interrupt `tts:stop` đi qua urgent lane bounded với thứ tự `urgent > normal control > audio`. Gate invalidate trước enqueue; urgent admission fail là session-integrity failure, fail-closed bằng root cancellation/writer shutdown escape path.
5. Reuse/generalize `AutoPcmRetention`, không thêm ring 500 ms. Capacity là `pre_roll_samples + confirmation_samples + vad_command_capacity * 960 + 960 + 512`; default hiện tại là `4,800 + 2,880 + 32*960 + 960 + 512 = 39,872` samples (xấp xỉ 2,492 giây ở 16 kHz). Snapshot `[start_sample - pre_roll, cursor)` phải xảy ra trước reset retention/VAD; `GenerationGate.invalidate(N)` là interruption linearization point.

ADR-0045 ghi nhận các trade-off khó đảo ngược này và supersede ADR-0010.

---

# 1. Behavior cần đạt

## 1.1. Flow mục tiêu

Khi AI đang phát TTS generation `N`:

```text
Assistant TTS generation N
        |
        | microphone Opus 60 ms
        v
Decode -> PCM 16 kHz mono
        |
        v
Echo-safe uplink?
        |
        +-- no --> không acoustic interrupt
        |
        +-- yes
             |
             v
          VAD detects real speech
             |
             v
      interrupt generation N
             |
             +--> invalidate outbound N
             +--> cancel LLM N
             +--> cancel TTS N
             +--> cancel queued/paced TTS N
             +--> send tts:stop
             |
             v
      create generation N+1
             |
             v
  feed SAME triggering PCM + pre-roll to ASR N+1
             |
        continue microphone
             |
          SpeechEnd
             |
             v
           ASR Final
             |
             v
           LLM N+1
             |
             v
           TTS N+1
```

Điểm bắt buộc:

1. Không mất phần đầu câu người dùng vừa nói chen.
2. Không có TTS generation cũ phát tiếp sau interrupt boundary.
3. Không có `llm`, `tts:start`, audio hoặc kết quả async cũ xuất hiện sau turn mới.
4. `manual` mode không tự acoustic interrupt.
5. Không có AEC/echo-safe capability thì không tự acoustic interrupt.
6. Explicit `abort` vẫn luôn được phép.

---

# 2. Xiaozhi đang xử lý như thế nào

## 2.1. Audio vẫn đi qua VAD khi assistant đang nói

Trong `receiveAudioHandle.py`:

```python
have_voice = conn.vad.is_vad(conn, pcm_frame)

if conn.client_aec and have_voice:
    if conn.client_is_speaking and conn.client_listen_mode != "manual":
        await handleAbortMessage(conn)

await conn.asr.receive_audio(conn, pcm_frame, have_voice)
```

Thứ tự rất quan trọng:

```text
VAD detects speech
    -> abort old output
    -> current pcm_frame vẫn đi vào ASR
```

Không được làm kiểu:

```rust
if speech_detected {
    interrupt();
    return; // SAI: mất frame đầu câu
}
```

## 2.2. Abort của Xiaozhi

`abortHandle.py` thực hiện:

```text
client_abort = true
clear TTS text/audio queues
reset audio pacing
send tts:stop
client_is_speaking = false
```

Sau đó producer cũ cũng tự dừng khi thấy `client_abort`.

## 2.3. Chống stale bằng `sentence_id`

Xiaozhi tạo `sentence_id` mới cho mỗi response mới và drop TTS cũ khi:

```text
message.sentence_id != conn.sentence_id
```

Rust không nên copy boolean/shared-ID theo đúng implementation Python. Equivalent an toàn hơn:

```text
Xiaozhi client_abort     -> Rust CancellationToken
Xiaozhi sentence_id      -> Rust GenerationId
Xiaozhi old-id filtering -> Rust GenerationGate
```

## 2.4. `listen:start` trong Xiaozhi

Xiaozhi xử lý `listen:start` chủ yếu bằng reset input/VAD/ASR state:

```python
if msg_json["state"] == "start":
    conn.reset_audio_states()
```

Nó không trực tiếp gọi abort ở handler này.

Vì vậy nếu cần parity sát Xiaozhi:

- `abort` = explicit interruption.
- `listen:start` = arm/reset capture mode.
- Nếu đang Speaking và sau đó speech thật xuất hiện, acoustic barge-in mới abort.

Quyết định đã chốt ở §0.1: `listen:start` không còn là explicit interrupt. Mọi implementation/document cũ gọi nó là interrupt phải được thay bằng `arm_listening(mode)` tách khỏi `interrupt_current_turn(reason)`.

---

# 3. Gap hiện tại của Rust

## 3.1. Không có runtime `Speaking`

Hiện tại:

```rust
pub enum SessionPhase {
    Ready,
    Listening,
    Processing,
    Closed,
}
```

Trong khi Phase 5 cần phân biệt ít nhất:

```rust
pub enum SessionPhase {
    Ready,
    Listening,
    Processing,
    Speaking,
    Closed,
}
```

`Speaking` phải bắt đầu khi `SpeechOutputEvent::Started` được chấp nhận cho generation hiện tại.

## 3.2. `on_binary()` drop toàn bộ mic khi không `Listening`

Hiện tại:

```rust
if self.phase == SessionPhase::Listening {
    ...
}
```

Do đó khi TTS đang phát, Rust không thể nhận voice barge-in.

## 3.3. ClientHello chưa lưu `features.aec`

`ClientHello` hiện chỉ có:

- `type`
- `version`
- `transport`
- `audio_params`

Cần tương thích field Xiaozhi:

```json
{
  "type": "hello",
  "features": {
    "aec": true
  }
}
```

## 3.4. `Realtime` parse được nhưng runtime chưa chạy

Protocol có:

```rust
pub enum ListenMode {
    Manual,
    Auto,
    Realtime,
}
```

Nhưng `replace_listening_mode()` hiện làm:

```rust
ListenMode::Realtime => self.phase = SessionPhase::Ready,
```

Đây là gap cần đóng. Xiaozhi xử lý mọi non-manual mode qua VAD/ASR path; `Realtime` là mode phù hợp nhất cho AEC/full-duplex barge-in.

## 3.5. Writer invalidation hiện đi qua bounded control queue

Hiện tại actor gửi:

```rust
OutboundMessage::InvalidateAudio(self.generation)
```

bằng `try_send()`.

Nếu control queue đầy thì invalidation có thể mất. Khi đó audio generation cũ đã nằm trong `audio_rx` vẫn có thể được gửi.

Generation invalidation không được phụ thuộc vào queue admission.

## 3.6. Chỉ binary audio có generation

Hiện tại:

```rust
pub enum OutboundMessage {
    Text(String),
    Binary { generation: u64, packet: Vec<u8> },
    InvalidateAudio(u64),
    Close(u16),
}
```

`llm`, `stt`, `tts:start` đều là `Text(String)` không có generation.

Sau abort, stale JSON control của old turn vẫn có thể nằm trong queue và được writer gửi.

## 3.7. VAD worker lease đang được reuse qua nhiều generation

`on_vad_event()` xác định current VAD bằng worker identity, không check generation vì Auto giữ worker pinned qua nhiều cycle.

Do đó late event từ semantic VAD cycle cũ có thể lọt sang generation mới.

Cần tách:

```text
WorkerIdentity = lifetime của worker lease
VadCycleId     = semantic listening/barge-in cycle
```

---

# 4. Kiến trúc Phase 5 đề xuất

## 4.1. GenerationId

Không dùng raw `u64` khắp code.

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GenerationId(u64);

impl GenerationId {
    pub fn next(self) -> Option<Self> {
        self.0.checked_add(1).map(Self)
    }
}
```

Mỗi turn semantic mới có generation riêng.

## 4.2. TurnContext

```rust
use tokio_util::sync::CancellationToken;

pub struct TurnContext {
    pub generation: GenerationId,
    pub cancel: CancellationToken,
}
```

Không reset một shared boolean như `client_abort=false`.

Turn `N+1` phải có token mới hoàn toàn; token của `N` vẫn permanently cancelled.

## 4.3. GenerationGate

Actor và writer dùng cùng một `Arc<GenerationGate>`.

Ví dụ:

```rust
use std::sync::atomic::{AtomicU64, Ordering};

pub struct GenerationGate {
    invalid_through: AtomicU64,
}

impl GenerationGate {
    pub fn new() -> Self {
        Self {
            invalid_through: AtomicU64::new(0),
        }
    }

    pub fn invalidate(&self, generation: GenerationId) {
        self.invalid_through
            .fetch_max(generation.0, Ordering::Release);
    }

    pub fn allows(&self, generation: GenerationId) -> bool {
        generation.0 > self.invalid_through.load(Ordering::Acquire)
    }
}
```

Nếu generation khởi tạo từ `0`, cần quy ước rõ để generation đầu tiên không bị coi invalid. Có thể bắt đầu từ `1`.

Alternative tốt hơn là gate lưu `current_generation`, writer chỉ chấp nhận equality. Dù dùng kiểu nào, invariant phải là:

```text
actor invalidates N trực tiếp trên shared gate
    BEFORE
writer được phép admit thêm turn payload N
```

Không gửi invalidation qua bounded mpsc queue.

## 4.4. Outbound message phải phân biệt Turn payload và Session control

Đề nghị:

```rust
pub enum OutboundMessage {
    TurnText {
        generation: GenerationId,
        text: String,
    },
    TurnAudio {
        generation: GenerationId,
        packet: Vec<u8>,
    },
    SessionControl(String),
    Close(u16),
}
```

Writer:

```rust
match message {
    OutboundMessage::TurnText { generation, text } => {
        if !gate.allows(generation) {
            continue;
        }
        send(text).await?;
    }
    OutboundMessage::TurnAudio { generation, packet } => {
        if !gate.allows(generation) {
            continue;
        }
        send(packet).await?;
    }
    OutboundMessage::SessionControl(text) => {
        send(text).await?;
    }
    ...
}
```

### Turn-scoped payload

Nên generation-tag:

- `stt`
- `llm`
- normal `tts:start`
- normal completion `tts:stop`
- audio binary

### Session-control payload

Không gate:

- abort `tts:stop`
- ServerHello
- Close
- protocol/session fatal control

Lý do: sau khi invalid generation `N`, abort `tts:stop` vẫn phải đi ra client.

---

# 5. ClientHello / AEC capability

## 5.1. Protocol struct

Thêm:

```rust
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ClientFeatures {
    #[serde(default)]
    pub aec: bool,
}
```

Và:

```rust
pub struct ClientHello {
    ...
    #[serde(default)]
    pub features: ClientFeatures,
}
```

Unknown feature vẫn phải được serde ignore để giữ compatibility.

## 5.2. Không đồng nhất `features.aec=true` với server-side AEC

Xiaozhi có một chi tiết quan trọng:

- `features.aec` bật `client_aec` và cho phép VAD-triggered interrupt.
- `_apply_aec()` ở server hiện chỉ được gọi trên path MQTT gateway có timestamp/reference audio.
- Binary WebSocket thông thường không có timestamp để server align playback reference với microphone.

Rust V1 hiện dùng raw Opus 60 ms binary và không có timestamp/header cho AEC alignment.

Vì vậy Phase 5 nên hỗ trợ trước:

```text
Client-side AEC / echo-suppressed uplink
```

và coi `features.aec=true` là capability assertion của client.

Nếu muốn an toàn hơn, thêm config:

```toml
[barge_in]
enabled = false
trust_client_aec_feature = false
```

Điều kiện acoustic barge-in:

```rust
fn acoustic_barge_in_enabled(&self) -> bool {
    self.barge_in_config.enabled
        && self.barge_in_config.trust_client_aec_feature
        && self.client_features.aec
        && matches!(self.listening_mode, Some(ListenMode::Auto | ListenMode::Realtime))
}
```

Nếu `features.aec=false` hoặc thiếu field:

```text
Speaking + mic => không acoustic abort
```

Explicit `abort` vẫn hoạt động.

---

# 6. State machine mới

## 6.1. States

```text
Ready
  |
  | listen:start
  v
Listening
  |
  | SpeechEnd / manual stop / detect
  v
Processing
  |
  | first TTS delivery Started
  v
Speaking
  |
  | normal Drained
  v
Listening (Auto/Realtime)
  or
Ready (Manual)
```

Acoustic interruption:

```text
Speaking N
  |
  | AEC-safe speech start
  v
interrupt N
  |
  v
Listening N+1, already capturing same utterance
```

Explicit abort:

```text
Processing/Speaking N
  |
  | abort
  v
invalidate/cancel N
  |
  +--> Auto/Realtime: re-arm listening policy
  +--> Manual: Ready unless explicit new listen:start follows
```

## 6.2. Khi nào set `Speaking`

Trong `drain_speech_output()`:

```rust
SpeechOutputEvent::Started => {
    if event_generation != self.current_generation() {
        return;
    }
    self.tts_started = true;
    self.phase = SessionPhase::Speaking;
    ...
}
```

Không set `Speaking` từ lúc LLM bắt đầu.

---

# 7. Realtime mode

`ListenMode::Realtime` hiện không được implement.

Đề nghị reuse gần như toàn bộ Auto pipeline:

```text
Manual:
    explicit capture boundaries
    không acoustic barge-in

Auto:
    VAD endpoint; chỉ watch Speaking nếu capture cycle đã được arm
    có thể acoustic barge-in nếu AEC assertion được server trust

Realtime:
    VAD endpoint; giữ capture/VAD armed xuyên Processing/Speaking
    acoustic barge-in nếu AEC assertion được server trust
```

Implementation không nên tạo provider khác. `Auto` và `Realtime` cùng Silero/ASR runtime; khác policy state machine.

Có thể thêm helper:

```rust
fn uses_vad(mode: ListenMode) -> bool {
    matches!(mode, ListenMode::Auto | ListenMode::Realtime)
}
```

---

# 8. VAD lifecycle cho barge-in

Đây là phần khó nhất.

## 8.1. Không dùng generation của WorkerIdentity làm semantic VAD cycle

VAD worker có thể pinned lâu hơn một turn. Vì vậy thêm:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VadCycleId(u64);
```

Command:

```rust
pub enum VadCommand {
    Push {
        cycle: VadCycleId,
        pcm: PcmF32Mono,
    },
    Reset {
        next_cycle: VadCycleId,
    },
    Close,
}
```

Event semantic:

```rust
pub enum VadWorkerEvent {
    Probability {
        identity: WorkerIdentity,
        cycle: VadCycleId,
        probability: VadProbability,
    },
    SpeechStart {
        identity: WorkerIdentity,
        cycle: VadCycleId,
        start_sample: u64,
    },
    SpeechEnd {
        identity: WorkerIdentity,
        cycle: VadCycleId,
        end_sample: u64,
    },
    ResetDone {
        identity: WorkerIdentity,
        cycle: VadCycleId,
    },
    ...
}
```

Cleanup events vẫn phải được xử lý kể cả cycle stale.

Semantic events chỉ nhận khi:

```rust
cycle == self.current_vad_cycle
```

## 8.2. Re-arm VAD để nghe barge-in

Sau khi user utterance cũ đã endpoint và ASR final đang/đã processing, pinned VAD phải được reset cho một cycle mới trước khi assistant Speaking.

Không được chờ tới `SpeechOutput::Drained`, vì như vậy lúc đang Speaking không có VAD cycle sạch để detect người dùng chen vào.

Đề nghị tách:

```text
Conversation turn lifecycle

khỏi

VAD detection cycle lifecycle
```

Ví dụ field:

```rust
current_vad_cycle: VadCycleId,
vad_ready: bool,
vad_purpose: VadPurpose,
```

```rust
pub enum VadPurpose {
    UserCapture,
    BargeInWatch,
}
```

Khi ASR của user cũ endpoint:

1. Stop feeding old ASR.
2. Reset VAD worker.
3. Bump `VadCycleId`.
4. Clear `VadSegmenter`.
5. Clear/reinitialize retention cursor for new cycle.
6. Khi `ResetDone`, set `vad_ready=true`.
7. Nếu phase là `Processing` hoặc `Speaking`, VAD cycle mới có thể dùng làm `BargeInWatch` nhưng chỉ khi AEC-safe.

Không tự chuyển `phase=Listening` chỉ vì VAD reset xong.

Hiện code:

```rust
if self.llm_operation.is_none() && !self.tts_started {
    self.phase = SessionPhase::Listening;
}
```

Cần bỏ coupling này và cho state transition owner quyết định rõ.

---

# 9. Audio ingress mới

## 9.1. Tách decode và routing

Hiện `on_binary()` gói tất cả vào `phase == Listening`.

Đề nghị:

```rust
pub fn on_binary(&mut self, payload: Vec<u8>) -> bool {
    let frame = match self.uplink_decoder.decode(&payload) {
        DecodeOutcome::Frame(frame) => frame,
        DecodeOutcome::Dropped(reason) => {
            ...
            return false;
        }
    };

    let pcm = PcmF32Mono::from_uplink(&frame);

    match self.phase {
        SessionPhase::Listening => self.route_listening_audio(frame, pcm),
        SessionPhase::Speaking if self.acoustic_barge_in_enabled() => {
            self.route_barge_in_audio(pcm)
        }
        SessionPhase::Processing if self.allow_processing_barge_in() => {
            // optional; không bắt buộc cho milestone đầu tiên
            self.route_barge_in_audio(pcm)
        }
        _ => false,
    }
}
```

Milestone đầu tiên nên chỉ cho acoustic barge-in ở `Speaking` để giảm state complexity.

## 9.2. Khi Speaking

Không push PCM trực tiếp vào ASR cũ.

Pipeline:

```text
PCM
 -> retention ring
 -> VAD worker BargeInWatch cycle
 -> khi SpeechStart:
      capture retained PCM
      interrupt old response
      create new generation
      open ASR new generation
      feed retained PCM
 -> các PCM sau đó:
      VAD + ASR new generation
```

---

# 10. Giữ triggering PCM và pre-roll

Đây là invariant quan trọng nhất để UX không mất chữ đầu.

Giả sử VAD xác nhận `SpeechStart { start_sample }` sau `min_speech_ms`.

Actor phải reuse/generalize `AutoPcmRetention`, với capacity:

```text
pre_roll_samples
+ confirmation_samples
+ vad_command_capacity * 960
+ current uplink frame (960)
+ rechunk slack (512)
```

Với default hiện tại: `4.800 + 2.880 + 32*960 + 960 + 512 = 39.872` samples, khoảng 2,492 giây. Retention bounded overwrite-oldest; nếu range snapshot thiếu onset, fail-closed thay vì tạo transcript mất prefix.

Khi barge-in được xác nhận:

```rust
let feed_start = start_sample.saturating_sub(self.pre_roll_samples);
let retained = self.auto_retention.range(feed_start)?;
```

Sau đó mới interrupt/reset ownership.

Không reset `auto_retention` trước khi lấy snapshot.

Pseudo-code:

```rust
fn on_barge_in_speech_start(&mut self, start_sample: u64) {
    let feed_start = start_sample.saturating_sub(self.pre_roll_samples);

    let retained = match self.auto_retention.range(feed_start) {
        Some(pcm) => pcm,
        None => {
            self.fail_closed();
            return;
        }
    };

    self.interrupt_current_turn(InterruptReason::AcousticBargeIn);

    self.start_new_barge_in_asr(retained);
}
```

Sau interrupt, các frame tiếp theo phải được push vào ASR generation mới.

---

# 11. Một primitive duy nhất cho interruption

Hiện logic cancel nằm rải ở:

- `replace_listening_mode`
- `restart_existing_auto_cycle`
- `abort_current_turn`
- `fail_closed`
- `cancel_speech_delivery`

Phase 5 nên gom semantic interruption vào một primitive.

```rust
pub enum InterruptReason {
    ExplicitAbort,
    AcousticBargeIn,
    ListenModeReplace,
    SessionFailure,
}
```

Ví dụ:

```rust
fn interrupt_current_turn(&mut self, reason: InterruptReason) -> GenerationId {
    let old = self.turn.generation;

    // 1. Linearization point: writer không được admit thêm old-turn payload.
    self.generation_gate.invalidate(old);

    // 2. Producer cancellation.
    self.turn.cancel.cancel();
    self.cancel_llm();
    self.speech_output.cancel();
    self.pending_audio = None;

    // 3. Native worker cleanup vẫn dùng ack/quarantine contract hiện có.
    self.cancel_asr_if_owned_by_old_turn();

    // 4. Release old active-turn admission.
    self.release_active_turn();

    // 5. Inform client if playback had started.
    if self.tts_started {
        self.enqueue_interrupt_tts_stop_or_fail_closed();
    }
    self.tts_started = false;

    // 6. Create a fresh turn identity.
    let next = old.next().expect("generation exhausted");
    self.turn = TurnContext {
        generation: next,
        cancel: self.session_cancel.child_token(),
    };

    next
}
```

Thứ tự quan trọng:

```text
invalidate writer gate
    BEFORE
producer cleanup hoàn tất
```

Vì cancellation upstream không thể thu hồi packet đã vào queue.

---

# 12. CancellationToken hierarchy

Đề nghị:

```text
Session root CancellationToken
    |
    +-- Turn N token
    |
    +-- Turn N+1 token
```

Disconnect:

```rust
session_cancel.cancel();
```

Turn interrupt:

```rust
turn.cancel.cancel();
```

Không dùng token để thay thế worker cleanup acknowledgement.

Native ASR/TTS worker vẫn phải giữ contract:

```text
CancelRequested
    -> worker safe-point cleanup
    -> Cancelled/CleanupAck
    -> lease reusable
```

Nếu timeout:

```text
quarantine worker
```

---

# 13. SpeechOutput phải generation-aware

Hiện event không mang generation:

```rust
pub enum SpeechOutputEvent {
    SegmentReady { text: String },
    Started,
    AudioPacket(Vec<u8>),
    Drained,
}
```

Đề nghị stream instance hoặc event phải gắn generation.

Option A:

```rust
pub enum SpeechOutputEvent {
    SegmentReady {
        generation: GenerationId,
        text: String,
    },
    Started {
        generation: GenerationId,
    },
    AudioPacket {
        generation: GenerationId,
        packet: Vec<u8>,
    },
    Drained {
        generation: GenerationId,
    },
}
```

Option B:

`SpeechOutput` giữ immutable `active_generation` và `poll()` trả struct wrapper.

Dù dùng option nào, không được lấy `self.generation` hiện tại tại thời điểm poll để gán cho packet có thể thuộc old TTS stream.

---

# 14. Writer contract

## 14.1. Không còn `InvalidateAudio` queue message

Xóa:

```rust
OutboundMessage::InvalidateAudio(u64)
```

Actor gọi trực tiếp:

```rust
self.generation_gate.invalidate(old_generation);
```

## 14.2. Gate cả JSON và binary

Writer phải kiểm tra gate ngay trước admission/send của mọi turn payload.

Pseudo-code:

```rust
if let Some(generation) = message.turn_generation() {
    if !gate.allows(generation) {
        continue;
    }
}

send_outbound(...).await;
```

## 14.3. Linearization contract

Không thể thu hồi một WebSocket frame đã thực sự được gửi xuống transport.

Do đó test/contract phải định nghĩa:

> Sau thời điểm `GenerationGate::invalidate(N)` hoàn tất, writer không được admit thêm bất kỳ turn payload generation `N` nào.

Packet đã được writer admit/send trước linearization point không thể recall.

## 14.4. `tts:stop` khi abort phải reliable

Hiện `try_send()` có thể fail nếu control queue full.

Không được silently bỏ `tts:stop` sau khi client đã nhận `tts:start`.

Các option:

### Option khuyến nghị

Tách `urgent_control_tx` cho:

- interrupt `tts:stop`
- close
- fatal protocol/session control

Writer `biased` select urgent trước normal control/audio.

Nếu urgent control admission thất bại, session phải fail/close thay vì tiếp tục không rõ client playback state.

### Không khuyến nghị

- retry loop blocking actor
- clear toàn bộ shared channel
- coi `try_send` failure là không sao

---

# 15. `listen:start` contract nên đổi để sát Xiaozhi

Hiện Rust `start_listening()` gọi `replace_listening_mode()` và việc đó cancel speech delivery ngay.

Contract đã chốt để sát Xiaozhi:

## `abort`

Luôn:

```text
interrupt current generation
```

## `listen:start`

Nên:

```text
set/replace listen mode
reset input/VAD semantic capture state
arm capture
```

Không abort assistant khi JSON tới.

Nếu đang Speaking:

- `manual`: không acoustic interrupt.
- `auto/realtime + AEC assertion đã được server trust`: frame voice thật tiếp theo sẽ trigger barge-in; Auto chỉ khi cycle đã arm, Realtime giữ cycle armed.
- Không AEC: TTS tiếp tục cho đến explicit abort hoặc normal stop.

Nếu cần giữ backward behavior của Rust cũ, có thể thêm compatibility config, nhưng Phase 5 spec phải chỉ có một behavior authoritative.

---

# 16. Barge-in algorithm chi tiết

## 16.1. Preconditions

Acoustic barge-in chỉ hợp lệ khi:

```rust
self.phase == SessionPhase::Speaking
&& self.client_features.aec
&& self.listening_mode != Some(ListenMode::Manual)
&& self.vad_ready
&& !self.barge_in_transitioning
```

## 16.2. Mỗi mic frame khi Speaking

```text
1. decode Opus -> PCM
2. retain PCM
3. push PCM vào current VAD BargeInWatch cycle
4. chưa SpeechStart -> không làm gì
5. SpeechStart -> perform transition một lần
```

## 16.3. Khi nhận SpeechStart

```text
old_generation = N
retained = [start-pre_roll .. now]

GenerationGate.invalidate(N)
cancel Turn N
cancel SpeechOutput N
send interrupt tts:stop
release ActiveTurn N

generation = N+1
open ASR stream N+1
feed retained
phase = Listening
auto_speech_active = true
```

VAD cycle có thể tiếp tục dùng cùng pinned worker nhưng phải chuyển semantic ownership sang cycle mới đúng cách.

## 16.4. Những frame sau SpeechStart

```text
retention.push(pcm)
VAD.Push(pcm)
ASR.Push(pcm)
```

## 16.5. SpeechEnd

Giống normal Auto flow:

```text
phase = Processing
acquire ActiveTurn permit
ASR Finish
ASR Final
commit user
LLM start
```

---

# 17. Không copy `clear_queues()` của Xiaozhi trực tiếp

Python Xiaozhi dùng shared synchronous queues nên `clear_queues()` phù hợp với kiến trúc của nó.

Rust đang có bounded Tokio channels và worker ownership/cleanup contract.

Không nên:

```text
drain arbitrary audio/control channels từ actor
```

Vì có thể race với generation mới.

Rust nên dùng:

```text
CancellationToken = stop producers
GenerationGate    = drop stale already-queued outputs
small bounded queues = bounded memory
worker cleanup ack = safe native resource reuse
```

Đây là semantic tương đương nhưng race-safe hơn.

---

# 18. Dialogue history khi bị interrupt

Rule hiện tại của Rust là tốt:

```text
User transcript -> commit khi ASR Final được chấp nhận
Assistant reply -> chỉ commit khi SpeechOutput Drained
```

Giữ nguyên.

Nếu assistant bị barge-in giữa câu:

```text
không commit toàn bộ generated_response như Delivered Assistant Response
```

Nếu sau này muốn lưu partial assistant text phục vụ context, đó phải là policy riêng, không được đánh dấu là delivered response.

---

# 19. Thay đổi file cụ thể

## 19.1. `src/protocol/client.rs`

Thêm:

- `ClientFeatures`
- `ClientHello.features`
- tests parse `features.aec=true/false/missing`

Không reject unknown feature.

## 19.2. `src/session/state.rs`

Thêm:

```rust
Speaking
```

Update transition tests.

## 19.3. `src/session/actor/mod.rs`

Thêm field dự kiến:

```rust
client_features: ClientFeatures,
turn: TurnContext,
generation_gate: Arc<GenerationGate>,
session_cancel: CancellationToken,
current_vad_cycle: VadCycleId,
vad_ready: bool,
vad_purpose: VadPurpose,
barge_in_transitioning: bool,
```

Có thể migration từng bước, không cần đổi tất cả cùng commit.

## 19.4. `src/session/actor/ingress.rs`

Refactor `on_binary` thành:

- decode chung
- Listening router
- Speaking barge-in watch router
- drop policy cho phase còn lại

Không early-return sau khi triggering speech được xác nhận nếu frame đó cần ASR.

## 19.5. `src/session/actor/listening.rs`

- Implement `Realtime`.
- Tách VAD worker lease khỏi VAD cycle.
- Thêm acoustic SpeechStart transition.
- Không để stale VAD cycle mở ASR generation mới.
- Xem lại semantics `listen:start` để sát Xiaozhi.

## 19.6. `src/session/actor/delivery.rs`

- `Started` -> `SessionPhase::Speaking`.
- SpeechOutput events generation-aware.
- `llm`, `tts:start`, audio gửi dưới turn-scoped outbound.
- `cancel_speech_delivery()` không còn gửi `InvalidateAudio` qua control queue.
- Dùng centralized interruption primitive.

## 19.7. `src/session/actor/lifecycle.rs`

- `fail_closed()` dùng same invalidation/cancellation primitive.
- session root token cancel on drop/disconnect.

## 19.8. `src/app/websocket.rs`

- Tạo `Arc<GenerationGate>` mỗi socket/session.
- Pass cùng Arc cho actor và writer.
- Writer gate TurnText + TurnAudio.
- Thêm urgent session-control path nếu cần.
- Bỏ local `invalidated_generation` được update từ `InvalidateAudio` message.

## 19.9. VAD worker command/event types

Nơi khai báo `VadCommand` / `VadWorkerEvent`:

- thêm `VadCycleId`
- semantic event mang cycle
- Reset ack correlate cycle

## 19.10. `speech_output/mod.rs`

- bind generation vào SpeechOutput stream/event.
- cancel reset toàn bộ pacing/buffer như hiện tại.
- không để old stream packet được relabel bằng new actor generation.

---

# 20. Thứ tự triển khai khuyến nghị

Không làm toàn bộ trong một commit.

## Commit 1 — GenerationGate thật

Mục tiêu:

- shared gate actor/writer
- gate JSON + audio
- remove `InvalidateAudio`
- stress queue stale output

Chưa cần acoustic barge-in.

Exit:

```text
explicit abort không còn stale audio/llm/tts:start
```

## Commit 2 — TurnContext + CancellationToken

Mục tiêu:

- session root token
- per-turn token
- centralized interrupt primitive
- worker cleanup ack giữ nguyên

Exit:

```text
old turn không thể được "un-cancel" khi new turn start
```

## Commit 3 — Runtime Speaking + protocol AEC feature

Mục tiêu:

- `SessionPhase::Speaking`
- parse `features.aec`
- no acoustic barge-in yet

Exit:

```text
state machine nhìn thấy chính xác khi playback active
```

## Commit 4 — VadCycleId + Realtime

Mục tiêu:

- semantic cycle isolation
- implement `Realtime`
- re-arm VAD independently from conversation phase

Exit:

```text
late old VAD event không thể mở ASR trong turn mới
```

## Commit 5 — Acoustic barge-in

Mục tiêu:

- Speaking + AEC + non-manual -> VAD-triggered interrupt
- retain triggering PCM/pre-roll
- start ASR generation mới

Exit:

```text
user nói chen không mất chữ đầu và old TTS dừng sạch
```

## Commit 6 — Compatibility/stress completion gate

Mục tiêu:

- deterministic tests
- Reference Client E2E
- real ZeroTTS + fake LLM
- optional real ESP/py-xiaozhi smoke

---

# 21. Tests bắt buộc

## 21.1. Protocol

1. Hello không có `features` -> accepted, `aec=false`.
2. `features.aec=false` -> accepted.
3. `features.aec=true` -> accepted.
4. Unknown feature -> ignored.

## 21.2. No-AEC behavior

Scenario:

```text
Auto/Realtime
TTS Speaking
mic speech arrives
features.aec=false
```

Expected:

```text
no generation change
no tts:stop
old TTS continues
mic không tạo ASR barge-in turn
```

## 21.3. Manual behavior

Scenario:

```text
Manual
features.aec=true
TTS Speaking
mic speech arrives
```

Expected:

```text
no acoustic interrupt
```

## 21.4. Acoustic barge-in happy path

Scenario:

```text
Realtime/Auto
features.aec=true
TTS N Speaking
speech detected
```

Expected order:

```text
GenerationGate invalidates N
tts:stop session-control
ASR N+1 starts with retained pre-roll
ASR receives continuing frames
ASR final
LLM N+1
TTS N+1
```

## 21.5. Trigger PCM preservation

Fixture có speech onset nằm trong packet gây VAD confirmation.

Assert ASR input contains:

```text
[start_sample - pre_roll, current_sample]
```

Không bị mất prefix.

## 21.6. Old audio queue already populated

- Fill bounded `audio_rx` với generation N packets.
- Trigger interrupt.
- Writer drains.

Assert:

```text
0 packet generation N admitted after gate invalidation
```

## 21.7. Control queue pressure

- Fill normal control queue.
- Trigger abort while TTS started.

Assert:

- stale N still blocked independently by GenerationGate.
- interrupt stop is delivered through urgent path hoặc connection fails closed.
- không silently continue old playback contract.

## 21.8. Stale JSON

Queue trước abort:

```text
llm N
llm N
tts:start N
```

Sau invalidate N:

```text
writer must drop all three if chưa admitted
```

Không chỉ test binary.

## 21.9. Late LLM/TTS events

Sau generation N bị interrupt:

- late LLM delta N
- late LLM Finished N
- late SpeechOutput packet N
- late Drained N

Expected:

```text
no user-visible output
no assistant history commit
```

## 21.10. Late VAD event

- old `VadCycleId=A`
- reset/rearm -> `B`
- late `SpeechStart(A)` arrives

Expected:

```text
ignored semantically
must not open ASR
```

## 21.11. Double interrupt

Hai SpeechStart/abort gần nhau:

```text
only one N -> N+1 transition
no double-release ActiveTurn
no panic
no duplicate tts:stop requirement beyond allowed idempotence
```

## 21.12. Disconnect during barge-in

Expected:

```text
session root cancellation fires
LLM/TTS/ASR semantic work cancelled
native cleanup ack still supervised
no worker premature reuse
```

---

# 22. Reference Client E2E scenario

Thêm scenario mới vào Rust Reference Client.

## Setup

- fake streaming LLM deterministic
- real ZeroTTS
- fake/deterministic or real Silero depending gate level
- canonical uplink Opus 16 kHz mono 60 ms
- `features.aec=true`
- mode `realtime` hoặc `auto`

## Script

```text
1. hello(features.aec=true)
2. listen:start realtime
3. send utterance A
4. receive stt A
5. receive tts:start A
6. wait until >= K TTS packets A received
7. while TTS still active, send speech utterance B
8. verify server sends tts:stop for A
9. continue sending B without restarting socket
10. endpoint B
11. receive stt B
12. receive tts:start B
13. receive TTS B
14. receive tts:stop B
```

Critical assertions:

```text
- socket không disconnect giữa A và B
- không cần reconnect
- không mất đầu utterance B
- sau interrupt boundary không có stale audio A
- B trở thành next LLM command
```

Thêm post-stop quiet period khoảng `250 ms` để chứng minh không còn packet old generation bị phát trễ.

---

# 23. Logging / telemetry cần thêm

Log structured fields:

```text
event=barge_in_detected
session_id
generation_old
generation_new
vad_cycle
listen_mode
aec=true
start_sample
pre_roll_samples
```

Interruption:

```text
event=turn_interrupted
reason=explicit_abort|acoustic_barge_in|mode_replace|failure
generation
had_tts_started
```

Writer stale drop:

```text
event=stale_outbound_dropped
generation
kind=text|audio
```

Không log raw PCM.

---

# 24. Config đề xuất

Tối thiểu:

```toml
[barge_in]
enabled = false
trust_client_aec_feature = false
```

Optional:

```toml
[barge_in]
enabled = false
trust_client_aec_feature = false
allow_during_processing = false
```

Milestone đầu nên để:

```text
allow_during_processing = false
```

chỉ interrupt khi actual `SessionPhase::Speaking`.

---

# 25. Server-side AEC: chưa nên nhét trực tiếp vào Phase 5 core

Rust V1 raw WebSocket Opus không có timestamp/reference metadata đủ mạnh để align microphone với TTS playback giống MQTT path của Xiaozhi.

Nếu cần server AEC thật sau này, tách thành provider/boundary riêng:

```rust
pub trait EchoCanceller: Send {
    fn push_reference(&mut self, timestamp: AudioTimestamp, playback: &Pcm16Mono);
    fn process_mic(
        &mut self,
        timestamp: AudioTimestamp,
        mic: Pcm16Mono,
    ) -> Result<Pcm16Mono, EchoCancelError>;
}
```

Cần protocol transport có:

- uplink timestamp
- downlink/playback timestamp hoặc sequence mapping
- bounded reference cache
- drift handling

Không port thẳng NumPy `_apply_aec()` của Xiaozhi vào Tokio actor hot path.

Cho Phase 5 hiện tại, client-side AEC + `features.aec` là đường ít rủi ro hơn.

---

# 26. Invariants để review code

PR chỉ được merge nếu giữ đủ các invariant sau.

## Generation

- Mỗi semantic turn có generation duy nhất.
- New turn không reuse cancellation token của old turn.
- Old generation không thể produce user-visible payload sau gate invalidation.

## Writer

- Gate không phụ thuộc bounded control queue.
- Text và audio đều gate.
- Abort `tts:stop` không bị gate bởi old generation.

## VAD

- Worker lease identity != semantic cycle identity.
- Late semantic VAD event stale bị drop.
- Cleanup event stale vẫn được xử lý.

## ASR

- Triggering PCM/pre-roll được feed vào new turn.
- ASR worker lease chỉ reusable sau cleanup ack.

## TTS

- `SpeechOutput::cancel()` reset packet/pacer/resampler state.
- Old TTS event không được relabel bằng current actor generation.

## State

- Mic normal capture chỉ ở `Listening`.
- Mic barge-in watch chỉ ở `Speaking` khi AEC-safe và non-manual.
- `manual` không acoustic interrupt.

## History

- Interrupted assistant response không được commit là Delivered Assistant Response.

---

# 27. Definition of Done cho Phase 5 mới

Phase 5 có thể đánh dấu hoàn tất khi toàn bộ điều kiện sau đạt:

1. Có runtime `Speaking`.
2. Có per-turn `CancellationToken`.
3. Có shared `GenerationGate` actor/writer.
4. Mọi turn-scoped outbound text/audio đều generation-tagged.
5. Explicit abort không phát stale output.
6. `features.aec` được parse tương thích Xiaozhi.
7. `Realtime` hoạt động bằng VAD/ASR thay vì trở về `Ready`.
8. Có `VadCycleId` hoặc cơ chế tương đương chống stale VAD semantic event.
9. Auto/Realtime + AEC cho phép voice barge-in khi Speaking.
10. Manual hoặc no-AEC không acoustic interrupt.
11. PCM gây interrupt và pre-roll không bị mất.
12. New utterance sau barge-in đi được trọn flow ASR -> LLM -> TTS trên cùng WebSocket.
13. Old assistant không commit history nếu bị interrupt.
14. Stress test queue/control/late worker events pass.
15. Reference Client E2E chứng minh không có stale audio sau interrupt + quiet period.

---

# 28. Checklist implementation ngắn

```text
[ ] Add ClientFeatures.aec
[ ] Add SessionPhase::Speaking
[ ] Add GenerationId newtype
[ ] Add TurnContext + CancellationToken
[ ] Add shared GenerationGate
[ ] Remove queue-based InvalidateAudio
[ ] Generation-tag TurnText + TurnAudio
[ ] Add urgent interrupt tts:stop path
[ ] Make SpeechOutput generation-aware
[ ] Add VadCycleId
[ ] Decouple VAD reset from SessionPhase transition
[ ] Implement ListenMode::Realtime
[ ] Re-arm VAD for Speaking barge-in watch
[ ] Retain PCM while barge-in watch is active
[ ] On SpeechStart: snapshot pre-roll before interruption/reset
[ ] Interrupt old generation once
[ ] Open ASR under new generation
[ ] Feed retained triggering PCM to new ASR
[ ] Continue incoming PCM to new ASR
[ ] Endpoint -> ASR Final -> LLM -> TTS normally
[ ] Manual/no-AEC regression tests
[ ] stale JSON/audio writer tests
[ ] late VAD cycle tests
[ ] Reference Client barge-in E2E
```

---

# 29. Kiến trúc cuối cùng mong muốn

```text
                              Voice Session
                                   |
                         +---------+----------+
                         |                    |
                    SessionActor          WS Writer
                         |                    |
                 TurnContext N              |
              generation + cancel           |
                         |                   |
                         +--- GenerationGate-+
                         |
       +-----------------+------------------+
       |                 |                  |
      VAD               ASR                LLM
 WorkerLease         StreamLease          Runtime
    + CycleId             |                  |
       |                  |                  v
       |                  |             SpeechOutput
       |                  |                  |
       |                  |              TTS Worker
       |                  |                  |
       +------ PCM -------+             Opus/Pacer
                                              |
                                              v
                                         TurnAudio N
                                              |
                                      GenerationGate
                                              |
                                              v
                                           WebSocket
```

Barge-in:

```text
Mic -> VAD BargeInWatch -> SpeechStart
                         |
                         v
                invalidate/cancel N
                         |
                         v
                  TurnContext N+1
                         |
             retained PCM + pre-roll
                         |
                         v
                       ASR N+1
```

Đây là cách giữ UX giống Xiaozhi nhưng tận dụng ownership/cancellation model của Rust để tránh các race vốn dễ xuất hiện nếu copy trực tiếp `client_abort` và `clear_queues()`.
