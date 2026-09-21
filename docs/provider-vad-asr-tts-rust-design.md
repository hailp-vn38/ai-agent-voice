# Thiết kế Providers VAD / ASR / TTS cho `voice-agent-server` bằng Rust

> **Repository:** `hailp-vn38/ai-agent-voice`  
> **Mục tiêu:** thêm kiến trúc provider/module có thể thay VAD, ASR và TTS bằng config mà không thay đổi `SessionActor`, WebSocket protocol hay dialogue core.  
> **Default implementation:** Silero VAD + Zipformer Streaming ASR + ZeroTTS.  
> **Runtime chính:** Rust, local inference. Không yêu cầu Python sidecar hoặc localhost HTTP service.  
> **Ngày chốt tài liệu:** 2026-09-21.

---

## 1. Phạm vi và quyết định kiến trúc

Project hiện đã có các contract quan trọng:

- WebSocket Protocol V1;
- uplink Opus 16 kHz mono 60 ms;
- downlink Opus 24 kHz mono 60 ms;
- `SessionActor` là owner duy nhất của state session;
- mọi queue phải bounded;
- cancellation theo `generation`;
- `SpeechOutput` sở hữu TTS → normalize/resample → Opus → pacing;
- provider không được biết WebSocket hoặc giữ mutable reference tới `SessionActor`.

Tài liệu này mở rộng kiến trúc đó bằng ba seam chính thức:

```text
VadProvider
AsrProvider
TtsProvider
```

Ba provider phải có thể đổi bằng `config.toml` + restart server.

### 1.1 Không làm plugin động `.so` trong V1

“Provider có thể thay đổi” trong V1 có nghĩa:

```text
binary được compile với nhiều adapter
        +
config chọn adapter khi startup
        +
provider/model load một lần khi startup
```

Không có yêu cầu:

```text
dlopen provider Rust ABI
hot-load .so
thay model giữa một Voice Session
```

Lý do: Rust không có stable ABI cho trait object, hot plugin làm tăng đáng kể complexity, deployment risk và khả năng leak resource.

Nếu sau này cần hot swap, thêm một layer `ArcSwap<ProviderSet>` ở application boundary, không thay contract provider.

---

## 2. Mục tiêu dependency

Core không được import type của model/runtime cụ thể.

### Cho phép

```text
session
  ↓
provider traits
  ↑
zipformer adapter
zerotts adapter
silero adapter
```

### Cấm

```text
SessionActor -> sherpa_onnx::OnlineRecognizer
SessionActor -> ort::Session
SpeechOutput -> ZeroTtsEngine concrete type
WebSocket writer -> TtsProvider
VadProvider -> Dialogue
AsrProvider -> WebSocket sender
TtsProvider -> GenerationGate
```

### Nguyên tắc

`SessionActor` điều phối business state.

Provider chỉ thực hiện inference và trả domain event/data.

---

# 3. Cây thư mục đề xuất

```text
crates/voice-agent-server/src/
├── app.rs
├── config.rs
├── lib.rs
│
├── audio/
│   ├── mod.rs
│   ├── pcm.rs
│   ├── opus.rs
│   ├── resample.rs
│   ├── frame_buffer.rs
│   ├── vad_segmenter.rs
│   └── pacer.rs
│
├── providers/
│   ├── mod.rs
│   ├── error.rs
│   ├── registry.rs
│   ├── set.rs
│   ├── capabilities.rs
│   │
│   ├── vad/
│   │   ├── mod.rs
│   │   ├── traits.rs
│   │   └── silero_sherpa.rs
│   │
│   ├── asr/
│   │   ├── mod.rs
│   │   ├── traits.rs
│   │   └── zipformer_sherpa.rs
│   │
│   └── tts/
│       ├── mod.rs
│       ├── traits.rs
│       └── zerotts_onnx/
│           ├── mod.rs
│           ├── config.rs
│           ├── engine.rs
│           ├── tokenizer.rs
│           ├── voice.rs
│           ├── generator.rs
│           ├── codec.rs
│           └── stream.rs
│
├── workers/
│   ├── mod.rs
│   ├── vad.rs
│   ├── asr.rs
│   └── tts.rs
│
├── session/
│   ├── actor.rs
│   ├── event.rs
│   ├── state.rs
│   └── turn.rs
│
└── speech_output/
    ├── mod.rs
    ├── command.rs
    └── worker.rs
```

## 3.1 Tại sao có `workers/` riêng?

Inference local là CPU-blocking.

Không chạy trực tiếp:

```rust
tokio::spawn(async move {
    model.run(...); // sai: block Tokio executor
});
```

Provider trait nên synchronous.

Worker bridge provider sang Tokio bằng:

- dedicated OS thread;
- bounded `tokio::sync::mpsc`;
- hoặc `spawn_blocking` có kiểm soát cho operation ngắn.

Với streaming ASR/TTS có state kéo dài, dedicated worker/session tốt hơn gọi `spawn_blocking` cho từng audio frame.

---

# 4. Domain audio chuẩn

Wire audio và provider audio phải tách rời.

## 4.1 Wire profile

```text
uplink:
  codec       Opus
  sample rate 16,000 Hz
  channels    1
  frame       60 ms

downlink:
  codec       Opus
  sample rate 24,000 Hz
  channels    1
  frame       60 ms
```

## 4.2 Internal PCM

Nên thêm type rõ ràng:

```rust
#[derive(Debug, Clone)]
pub struct Pcm16Mono {
    pub sample_rate_hz: u32,
    pub samples: Vec<i16>,
}

#[derive(Debug, Clone)]
pub struct PcmF32Mono {
    pub sample_rate_hz: u32,
    pub samples: Vec<f32>,
}
```

Không truyền `Vec<u8>` PCM trong provider contract.

### Conversion helper

```rust
pub fn i16_to_f32(samples: &[i16]) -> Vec<f32> {
    samples
        .iter()
        .map(|&x| x as f32 / i16::MAX as f32)
        .collect()
}
```

Provider ASR/VAD thường dùng normalized `f32`.

TTS trả `f32`.

---

# 5. ProviderSet và lifecycle

Model load một lần trong startup.

```rust
pub struct ProviderSet {
    pub vad: Arc<dyn VadProvider>,
    pub asr: Arc<dyn AsrProvider>,
    pub tts: Arc<dyn TtsProvider>,
}
```

`AppState`:

```rust
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub providers: Arc<ProviderSet>,
}
```

Startup:

```text
load config
    ↓
validate config
    ↓
build ProviderRegistry
    ↓
load VAD
load ASR
load TTS
    ↓
warmup
    ↓
start Axum server
```

Nếu provider bắt buộc không load được, server **fail fast** trước khi bind public socket.

Không chờ đến request đầu tiên mới báo:

```text
model file missing
tokenizer mismatch
unsupported execution provider
voice missing
invalid ONNX graph
```

---

# 6. Registry / factory

Adapter-specific code chỉ xuất hiện trong registry/factory.

## 6.1 Provider key

Khuyến nghị tên ổn định:

```text
VAD:
  silero_sherpa

ASR:
  zipformer_sherpa

TTS:
  zerotts_onnx
```

Tên không chứa version model.

Model version là config/artifact concern.

## 6.2 Factory contract

```rust
pub trait VadFactory: Send + Sync {
    fn build(
        &self,
        config: &VadProviderConfig,
    ) -> Result<Arc<dyn VadProvider>, ProviderLoadError>;
}

pub trait AsrFactory: Send + Sync {
    fn build(
        &self,
        config: &AsrProviderConfig,
    ) -> Result<Arc<dyn AsrProvider>, ProviderLoadError>;
}

pub trait TtsFactory: Send + Sync {
    fn build(
        &self,
        config: &TtsProviderConfig,
    ) -> Result<Arc<dyn TtsProvider>, ProviderLoadError>;
}
```

## 6.3 Static registry là đủ cho V1

```rust
pub struct ProviderRegistry {
    vad: HashMap<&'static str, Arc<dyn VadFactory>>,
    asr: HashMap<&'static str, Arc<dyn AsrFactory>>,
    tts: HashMap<&'static str, Arc<dyn TtsFactory>>,
}
```

Register:

```rust
let registry = ProviderRegistry::builder()
    .register_vad("silero_sherpa", SileroVadFactory)
    .register_asr("zipformer_sherpa", ZipformerAsrFactory)
    .register_tts("zerotts_onnx", ZeroTtsFactory)
    .build();
```

Sau này thêm model:

```rust
.register_vad("ten_vad_sherpa", TenVadFactory)
.register_asr("whisper_onnx", WhisperFactory)
.register_tts("piper", PiperFactory)
```

Không đổi caller.

---

# 7. Config schema

Nên phân biệt:

1. **core policy**: semantics server muốn giữ ổn định;
2. **adapter config**: tham số riêng model/runtime.

## 7.1 VAD

```toml
[vad]
adapter = "silero_sherpa"

# Core segmentation policy.
min_speech_ms = 180
end_silence_ms = 600
pre_roll_ms = 300
max_utterance_ms = 30000

[vad.adapter_config]
model = "models/vad/silero_vad.onnx"
threshold = 0.50
sample_rate_hz = 16000
window_samples = 512
num_threads = 1
provider = "cpu"
```

## 7.2 ASR

```toml
[asr]
adapter = "zipformer_sherpa"
timeout_ms = 15000
partial_emit_interval_ms = 200

[asr.adapter_config]
encoder = "models/asr/zipformer-30m-vi/encoder.onnx"
decoder = "models/asr/zipformer-30m-vi/decoder.onnx"
joiner = "models/asr/zipformer-30m-vi/joiner.onnx"
tokens = "models/asr/zipformer-30m-vi/tokens.txt"

sample_rate_hz = 16000
num_threads = 2
provider = "cpu"
decoding_method = "greedy_search"
enable_internal_endpoint = false
```

`enable_internal_endpoint = false` là default vì endpoint semantics của server thuộc Silero + `VadSegmenter`.

Không để sherpa endpoint và server VAD đồng thời quyết định utterance boundary, nếu không có thể tạo hai state machine cạnh tranh nhau.

## 7.3 TTS

```toml
[tts]
adapter = "zerotts_onnx"
timeout_ms = 15000

[tts.adapter_config]
model_dir = "models/tts/zerotts"
voice = "maichi"
execution_provider = "cpu"

intra_op_num_threads = 4
codec_intra_op_num_threads = 2
warmup = true

cfg_scale = 1.0
audio_temperature = 0.8
audio_topk = 25
audio_topp = 0.95
audio_repetition_penalty = 1.2
eoa_extra_frames = 1

max_frames = 1500
```

## 7.4 Rust generic config

```rust
#[derive(Clone, Debug, Deserialize)]
pub struct VadConfig {
    pub adapter: String,

    pub min_speech_ms: u32,
    pub end_silence_ms: u32,
    pub pre_roll_ms: u32,
    pub max_utterance_ms: u32,

    #[serde(default)]
    pub adapter_config: toml::Table,
}

#[derive(Clone, Debug, Deserialize)]
pub struct AsrConfig {
    pub adapter: String,
    pub timeout_ms: u64,
    pub partial_emit_interval_ms: u64,

    #[serde(default)]
    pub adapter_config: toml::Table,
}

#[derive(Clone, Debug, Deserialize)]
pub struct TtsConfig {
    pub adapter: String,
    pub timeout_ms: u64,

    #[serde(default)]
    pub adapter_config: toml::Table,
}
```

Mỗi factory tự parse `adapter_config` sang typed struct riêng.

Ví dụ:

```rust
#[derive(Debug, Deserialize)]
struct ZipformerConfig {
    encoder: PathBuf,
    decoder: PathBuf,
    joiner: PathBuf,
    tokens: PathBuf,

    #[serde(default = "default_sample_rate")]
    sample_rate_hz: u32,

    #[serde(default = "default_threads")]
    num_threads: i32,

    #[serde(default = "default_provider")]
    provider: String,

    #[serde(default = "default_decoding_method")]
    decoding_method: String,

    #[serde(default)]
    enable_internal_endpoint: bool,
}
```

---

# 8. Provider capabilities

Không nên để caller suy đoán capability bằng tên adapter.

```rust
#[derive(Debug, Clone)]
pub struct VadCapabilities {
    pub sample_rate_hz: u32,
    pub preferred_window_samples: usize,
}

#[derive(Debug, Clone)]
pub struct AsrCapabilities {
    pub sample_rate_hz: u32,
    pub streaming: bool,
    pub partial_results: bool,
}

#[derive(Debug, Clone)]
pub struct TtsCapabilities {
    pub streaming: bool,
    pub output_sample_rate_hz: u32,
    pub channels: u8,
}
```

Provider:

```rust
pub trait AsrProvider: Send + Sync {
    fn name(&self) -> &'static str;
    fn capabilities(&self) -> AsrCapabilities;
    fn open(
        &self,
        request: AsrStartRequest,
    ) -> Result<Box<dyn AsrSession>, AsrError>;
}
```

---

# 9. VAD provider contract

VAD provider nên trả **speech probability / score**, còn utterance segmentation thuộc core.

Điều này rất quan trọng.

Nếu provider tự quyết định `SpeechStart/SpeechEnd`, khi đổi Silero → TenVAD, semantics `min_speech_ms/end_silence_ms/pre_roll_ms` có thể thay đổi.

## 9.1 Contract

```rust
#[derive(Debug, Clone)]
pub struct VadFrame {
    pub probability: f32,
    pub samples: usize,
}

pub trait VadProvider: Send + Sync {
    fn name(&self) -> &'static str;

    fn capabilities(&self) -> VadCapabilities;

    fn open(&self) -> Result<Box<dyn VadSession>, VadError>;
}

pub trait VadSession: Send {
    fn push_pcm(
        &mut self,
        pcm: &[f32],
    ) -> Result<Vec<VadFrame>, VadError>;

    fn reset(&mut self) -> Result<(), VadError>;
}
```

Một `VadSession` tương ứng một Voice Session/microphone stream.

Không share recurrent state Silero giữa hai device.

## 9.2 Core `VadSegmenter`

```text
VadProvider probability
      ↓
VadSegmenter
      ├── min_speech_ms
      ├── end_silence_ms
      ├── max_utterance_ms
      └── threshold/hysteresis policy
      ↓
VadEvent::SpeechStart
VadEvent::SpeechEnd
```

Pre-roll thuộc audio capture layer:

```text
decoded PCM
   ├──→ PCM ring buffer (pre-roll)
   └──→ VAD
```

Khi `SpeechStart`:

```text
open ASR stream
feed pre-roll
feed current/live PCM
```

---

# 10. Silero VAD implementation bằng Rust

## 10.1 Default backend khuyến nghị

Dùng `sherpa-onnx` cho Silero VAD và Zipformer ASR để giảm số lượng custom inference loop cần duy trì.

Current Rust `sherpa-onnx` API có:

```text
VoiceActivityDetector
SileroVadModelConfig
VadModelConfig
```

Silero upstream hỗ trợ 8 kHz và 16 kHz. Với project này profile canonical là 16 kHz.

Window khuyến nghị:

```text
512 samples @ 16 kHz
= 32 ms
```

Đây là kích thước phù hợp với Silero và cũng được sherpa-onnx khuyến nghị.

## 10.2 Rechunk 60 ms → 32 ms

Một uplink packet:

```text
60 ms * 16,000 = 960 samples
```

Silero cần:

```text
512 samples
```

Không gọi Silero mỗi WebSocket packet.

Phải có accumulator:

```rust
pub struct FixedFrameBuffer {
    frame_samples: usize,
    pending: VecDeque<f32>,
}
```

Flow:

```text
Opus packet #1 -> 960 samples
  consume 512
  remain 448

Opus packet #2 -> +960
  pending 1408
  consume 512
  consume 512
  remain 384

...
```

Transport frame boundary không được trở thành model inference boundary.

## 10.3 Option A: sherpa `VoiceActivityDetector`

Nếu dùng sherpa VAD high-level API, adapter vẫn phải che implementation khỏi core.

Pseudo-code:

```rust
pub struct SileroVadProvider {
    config: SileroVadConfig,
}

pub struct SileroVadSession {
    detector: sherpa_onnx::VoiceActivityDetector,
    rechunker: FixedFrameBuffer,
}
```

Tuy nhiên cần chú ý: high-level sherpa VAD có segmentation riêng.

Nếu project muốn giữ **100%** segmentation policy trong core, có hai cách:

1. dùng direct Silero ONNX adapter và lấy probability;
2. dùng API lower-level tương ứng nếu sherpa expose probability ở version đang pin.

Nếu `VoiceActivityDetector` chỉ trả segment, hãy ưu tiên direct ONNX cho VAD thay vì duplicate segmentation semantics.

### Khuyến nghị cho project này

**Provider boundary phải là probability-level.**

Vì vậy implementation thực tế nên chọn:

```text
Silero ONNX trực tiếp qua ort
```

nếu sherpa Rust API không cung cấp raw probability ổn định.

Điều này không thay provider trait.

---

# 11. Silero ONNX trực tiếp bằng `ort`

Silero V5 16 kHz frame contract phổ biến:

```text
audio frame     512 samples
context          64 samples
state       recurrent tensor
sample rate  int64 = 16000
```

Upstream implementation giữ:

```text
state
context
last sample rate
```

giữa các lần inference.

## 11.1 State per session

```rust
pub struct SileroSession {
    session: Arc<Mutex<ort::session::Session>>,
    state: SileroState,
    context: [f32; 64],
    rechunker: FixedFrameBuffer,
}
```

Không dùng một mutable recurrent state global.

Nếu `ort::Session::run` yêu cầu mutable access, có hai lựa chọn:

- mỗi VAD stream giữ session riêng vì model nhỏ;
- hoặc pool session inference và state tensor nằm ngoài model session.

VAD rất nhỏ nên ưu tiên correctness trước tối ưu reuse.

## 11.2 Reset

Reset khi:

- Voice Session mới;
- sample rate thay đổi;
- listen cycle mới nếu semantics yêu cầu;
- session disconnect;
- explicit provider reset.

Không reset vì một packet Opus decode fail.

## 11.3 Threshold

Provider chỉ xuất probability.

Threshold có thể đặt trong `VadSegmenter`, ví dụ:

```text
speech enter threshold = 0.50
speech exit threshold  = 0.35
```

Hysteresis tránh chattering.

Không bắt buộc phải dùng hai threshold trong V1, nhưng API core nên cho phép.

---

# 12. ASR provider contract

ASR phải hỗ trợ true streaming.

Contract cũ:

```rust
async fn transcribe(AsrRequest) -> AsrResult
```

không phù hợp với streaming Zipformer.

## 12.1 Contract mới

```rust
#[derive(Debug, Clone)]
pub struct AsrStartRequest {
    pub sample_rate_hz: u32,
    pub language: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AsrPartial {
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct AsrResult {
    pub text: String,
}

#[derive(Debug, Clone)]
pub enum AsrEvent {
    Partial(AsrPartial),
    Final(AsrResult),
}

pub trait AsrProvider: Send + Sync {
    fn name(&self) -> &'static str;

    fn capabilities(&self) -> AsrCapabilities;

    fn open(
        &self,
        request: AsrStartRequest,
    ) -> Result<Box<dyn AsrSession>, AsrError>;
}

pub trait AsrSession: Send {
    fn push_pcm(
        &mut self,
        pcm: &[f32],
    ) -> Result<Vec<AsrEvent>, AsrError>;

    fn finish(
        &mut self,
    ) -> Result<AsrResult, AsrError>;

    fn cancel(&mut self);
}
```

## 12.2 Adapter offline vẫn dùng được

Whisper offline adapter tương lai:

```text
open()
push_pcm() -> buffer only
push_pcm() -> buffer only
finish()   -> full inference
```

Zipformer:

```text
open()
push_pcm() -> decode ready frames -> Partial
push_pcm() -> decode ready frames -> Partial
finish()   -> input_finished + drain -> Final
```

Caller không đổi.

---

# 13. Zipformer Streaming ASR bằng Rust

## 13.1 Model mặc định

Dùng streaming checkpoint:

```text
hynt/Zipformer-30M-RNNT-Streaming-6000h
```

Không dùng offline checkpoint:

```text
hynt/Zipformer-30M-RNNT-6000h
```

cho low-latency streaming pipeline.

Model streaming hiện có variant:

```text
chunk-16-left-128
chunk-32-left-128
chunk-64-left-128
```

Default đề xuất:

```text
chunk-32-left-128
```

Artifacts:

```text
encoder-epoch-31-avg-11-chunk-32-left-128.fp16.onnx
decoder-epoch-31-avg-11-chunk-32-left-128.fp16.onnx
joiner-epoch-31-avg-11-chunk-32-left-128.fp16.onnx
```

Đóng gói local thành:

```text
models/asr/zipformer-30m-vi/
├── encoder.onnx
├── decoder.onnx
├── joiner.onnx
└── tokens.txt
```

### Lưu ý `config.json`

Repo model có một artifact tên `config.json` có nội dung token mapping dạng text ở một số revision/history.

Không phụ thuộc vào tên đó.

Build/deploy script phải chuẩn hóa token artifact thành `tokens.txt`.

---

# 14. Zipformer runtime: `sherpa-onnx`

Rust crate chính thức hiện cung cấp:

```text
OnlineRecognizer
OnlineRecognizerConfig
OnlineTransducerModelConfig
OnlineStream
```

Pattern:

```rust
use sherpa_onnx::{OnlineRecognizer, OnlineRecognizerConfig};

let mut config = OnlineRecognizerConfig::default();

config.model_config.transducer.encoder =
    Some("models/asr/zipformer/encoder.onnx".into());

config.model_config.transducer.decoder =
    Some("models/asr/zipformer/decoder.onnx".into());

config.model_config.transducer.joiner =
    Some("models/asr/zipformer/joiner.onnx".into());

config.model_config.tokens =
    Some("models/asr/zipformer/tokens.txt".into());

config.model_config.num_threads = 2;
config.model_config.provider = Some("cpu".into());

config.decoding_method = Some("greedy_search".into());
config.enable_endpoint = false;

let recognizer = OnlineRecognizer::create(&config)?;
```

> API cụ thể phải được pin bằng Cargo.lock. Đoạn trên thể hiện current sherpa-onnx Rust API family, không dùng làm lý do bỏ contract wrapper của project.

## 14.1 Engine shared, stream per utterance

```rust
pub struct ZipformerProvider {
    recognizer: Arc<OnlineRecognizer>,
    sample_rate_hz: u32,
}

pub struct ZipformerSession {
    recognizer: Arc<OnlineRecognizer>,
    stream: OnlineStream,
    last_partial: String,
    cancelled: bool,
}
```

Nếu upstream type không `Sync` theo version đã pin, không ép `unsafe impl Sync`.

Thay vào đó đặt recognizer trong dedicated ASR worker và chỉ truyền command qua channel.

Không vượt qua thread-safety guarantee của FFI.

## 14.2 Push audio

Pseudo-code:

```rust
fn push_pcm(
    &mut self,
    samples: &[f32],
) -> Result<Vec<AsrEvent>, AsrError> {
    self.stream.accept_waveform(16_000, samples);

    while self.recognizer.is_ready(&self.stream) {
        self.recognizer.decode(&self.stream);
    }

    let result = self.recognizer
        .get_result(&self.stream)
        .ok_or(AsrError::MissingResult)?;

    if result.text != self.last_partial {
        self.last_partial.clone_from(&result.text);

        return Ok(vec![
            AsrEvent::Partial(AsrPartial {
                text: result.text,
            })
        ]);
    }

    Ok(Vec::new())
}
```

## 14.3 Finish

```rust
fn finish(&mut self) -> Result<AsrResult, AsrError> {
    self.stream.input_finished();

    while self.recognizer.is_ready(&self.stream) {
        self.recognizer.decode(&self.stream);
    }

    let result = self.recognizer
        .get_result(&self.stream)
        .ok_or(AsrError::MissingResult)?;

    Ok(AsrResult {
        text: result.text.trim().to_owned(),
    })
}
```

## 14.4 Partial result coalescing

Không gửi partial cho actor mỗi decode tick.

Core config:

```text
partial_emit_interval_ms = 200
```

Worker chỉ phát partial nếu:

```text
text changed
AND
elapsed >= partial_emit_interval
```

Partial STT không được commit vào dialogue.

Chỉ `AsrFinal` hợp lệ mới commit user message.

---

# 15. VAD + ASR streaming flow

```text
                  ┌─────────────────────┐
Opus uplink ────→ │ OpusDecoder 16 kHz │
                  └─────────┬───────────┘
                            │ PCM16
                  ┌─────────┴──────────┐
                  │                    │
                  ▼                    ▼
          Pre-roll ring         f32 conversion
                  │                    │
                  │               VadProvider
                  │                    │
                  │              probabilities
                  │                    │
                  │              VadSegmenter
                  │                    │
                  │        ┌───────────┴───────────┐
                  │        │                       │
                  │   SpeechStart              SpeechEnd
                  │        │                       │
                  └───────→│                       │
                           ▼                       ▼
                   open AsrSession          AsrSession.finish
                           │                       │
                           ├── feed pre-roll      │
                           ├── feed live PCM      │
                           ├── partial            │
                           └──────────────────────→│
                                                   ▼
                                               AsrFinal
```

## 15.1 Active Turn semantics

Streaming ASR bắt đầu trước utterance end.

Không giữ `ActiveTurnPermit` cho toàn thời gian người dùng đang nói nếu permit được định nghĩa cho expensive post-utterance work.

Tách:

```text
AsrStreamLease
ActiveTurnPermit
```

Đề xuất:

```text
SpeechStart
  → acquire ASR stream slot

SpeechEnd + valid final
  → acquire ActiveTurnPermit
  → LLM
  → TTS
  → terminal
```

Nếu cần limit tổng CPU ASR, có semaphore riêng:

```text
max_asr_streams
```

---

# 16. TTS provider contract

TTS provider không trả Opus.

Provider output là PCM.

```rust
#[derive(Debug, Clone)]
pub struct TtsRequest {
    pub text: String,
    pub voice: Option<String>,
}

#[derive(Debug)]
pub struct TtsPcmChunk {
    pub sample_rate_hz: u32,
    pub channels: u8,
    pub samples: Vec<f32>,
}

pub trait TtsProvider: Send + Sync {
    fn name(&self) -> &'static str;

    fn capabilities(&self) -> TtsCapabilities;

    fn synthesize(
        &self,
        request: TtsRequest,
    ) -> Result<Box<dyn TtsStream>, TtsError>;
}

pub trait TtsStream: Send {
    fn next_chunk(
        &mut self,
    ) -> Result<Option<TtsPcmChunk>, TtsError>;

    fn cancel(&mut self);
}
```

`None` nghĩa provider stream hoàn tất bình thường.

Provider completion với non-empty input nhưng không tạo chunk hợp lệ:

```text
tts_empty_audio
```

và phải map thành `SpeechOutputEvent::Failed`.

---

# 17. SpeechOutput không phụ thuộc ZeroTTS

Flow bắt buộc:

```text
TtsProvider
    ↓
TtsPcmChunk
    ↓
validate sample format
    ↓
resample -> 24 kHz
    ↓
PCM frame accumulator
    ↓
1440 samples = 60 ms @ 24 kHz
    ↓
Opus encoder
    ↓
AudioPacer
    ↓
SpeechOutputEvent::AudioPacket
```

`tts:start` chỉ được phát khi packet Opus đầu tiên đã sẵn sàng.

`Drained` chỉ được phát sau:

```text
FinishInput
+
mọi text segment synthesized
+
mọi PCM drained
+
Opus final packet paced
```

---

# 18. ZeroTTS implementation bằng Rust

## 18.1 Tại sao có thể port trực tiếp

ZeroTTS inference upstream không cần PyTorch.

Hot inference path sử dụng:

```text
ONNX Runtime
tokenizer
array operations
```

Upstream có `docs/RUNTIME.md` mô tả graph contract để implement runtime bằng ngôn ngữ khác.

Vì vậy V1 không cần:

```text
Python
FastAPI
subprocess
HTTP localhost
```

## 18.2 Model pipeline

```text
text
 ↓
NFC + whitespace normalize
 ↓
BPE tokenizer
 ↓
text_encoder.onnx
 ↓
prefix_step.onnx
 ↓
local_frame_decode.onnx
 ↓
audio code frame
 ↓
codec/decode_step.onnx
 ↓
PCM 48 kHz f32
```

Mỗi model frame tương ứng:

```text
1 / 12.5 s = 80 ms audio
```

Generation hot loop có hai ONNX call/model frame:

```text
local_frame_decode
prefix_step
```

codec `decode_step` chạy streaming trên generated codes.

---

# 19. `zerotts_onnx` module layout

```text
providers/tts/zerotts_onnx/
├── mod.rs
├── config.rs
├── engine.rs
├── tokenizer.rs
├── voice.rs
├── state.rs
├── generator.rs
├── codec.rs
└── stream.rs
```

## 19.1 `engine.rs`

Sở hữu immutable/shared model resources:

```rust
pub struct ZeroTtsEngine {
    text_encoder: OrtSessionHandle,
    prefix_step: OrtSessionHandle,
    frame_decoder: OrtSessionHandle,

    codec: Arc<MossCodecEngine>,

    tokenizer: Arc<ZeroTtsTokenizer>,
    voices: Arc<VoiceStore>,
    model_config: ZeroTtsModelConfig,
}
```

Không lưu generation KV cache trong `ZeroTtsEngine`.

## 19.2 `state.rs`

Per synthesis stream:

```rust
pub struct ZeroTtsState {
    packed_kv: Tensor,
    full_valid: Tensor,
    cross_kv: Tensor,
    text_valid: Tensor,

    seen_mask: Vec<bool>,

    codec_state: MossCodecStreamingState,

    frame_index: usize,
    tail_remaining: Option<u32>,

    cancelled: bool,
}
```

---

# 20. ZeroTTS model artifacts

Expected local model directory:

```text
models/tts/zerotts/
├── config.json
├── tokenizer.json
├── null_voice_emb.npy
├── voices/
│   ├── index.json
│   └── maichi/
│       ├── voice.npz
│       └── ...
└── onnx/
    ├── text_encoder.onnx
    ├── prefix_step.onnx
    ├── local_frame_decode.onnx
    └── codec/
        ├── decode_full.onnx
        ├── decode_step.onnx
        └── codec_browser_onnx_meta.json
```

Tên file thực tế phải validate khi startup.

Không tự động fallback silent sang graph cũ.

---

# 21. ZeroTTS tokenizer contract

Tokenizer là model contract, không phải UI helper.

Upstream normalization trước BPE:

```text
Unicode NFC
+
collapse mọi whitespace run thành một space
```

Không lowercase.

Không tự xóa punctuation.

Special token IDs hiện được model pin:

```text
<pad>  = 0
<bos>  = 1
<eot>  = 2
<soa>  = 3
<slot> = 4
<eoa>  = 5
<en>   = 6
<vi>   = 7
```

Rust loader phải validate IDs này khi startup.

Nếu tokenizer không match weights:

```text
fail startup
```

Không synthesize với tokenizer sai.

## 21.1 Rust tokenizer

Dùng Hugging Face `tokenizers` crate.

Pseudo-code:

```rust
pub struct ZeroTtsTokenizer {
    inner: tokenizers::Tokenizer,
}

impl ZeroTtsTokenizer {
    pub fn encode(&self, text: &str) -> Result<Vec<i64>, TtsError> {
        let normalized = normalize_nfc_and_ws(text);
        let encoding = self.inner.encode(normalized, false)?;

        let mut ids = Vec::with_capacity(encoding.len() + 2);
        ids.push(1); // <bos>
        ids.extend(encoding.get_ids().iter().map(|&x| x as i64));
        ids.push(2); // <eot>

        Ok(ids)
    }
}
```

NFC normalization cần crate phù hợp, ví dụ `unicode-normalization`.

---

# 22. ZeroTTS voice contract

Voice là latent:

```text
shape:
  (1, n_voice_queries, d_model)

current published example:
  n_voice_queries = 10
```

Provider request chỉ expose tên voice:

```rust
TtsRequest {
    text,
    voice: Some("maichi".into()),
}
```

`VoiceStore` resolve tên → latent.

Voice latent không được đi vào `SessionActor`.

## 22.1 V1 không implement voice cloning

Current public ZeroTTS release có thể load precomputed voice pack nhưng không ship voice encoder tạo latent từ WAV.

V1:

```text
load preset/precomputed voice = supported
clone voice từ raw reference WAV = out of scope
```

---

# 23. ZeroTTS graph contracts

## 23.1 `text_encoder.onnx`

Inputs:

```text
text_ids     (B, L) int64
txt_lengths  (B)    int64
```

Outputs:

```text
text_states
text_valid
soa_embed
cross_kv
```

`cross_kv` phải giữ suốt utterance.

Không recompute mỗi frame.

## 23.2 `prefix_step.onnx`

Hai mode cùng một graph:

```text
cold start:
  [voice | soa]

frame step:
  generated frame code
```

Vị trí logic:

```text
voice queries: 0 .. V-1
<soa>:          V
frame t:        V + 1 + t
```

Off-by-one ở `new_pos` không crash nhưng làm giảm chất lượng.

Phải có unit test.

## 23.3 `local_frame_decode.onnx`

Per frame:

```text
global_hidden
sampling params
seen_mask
random draws
```

Outputs:

```text
is_eoa
codes (1, K)
```

Sau mỗi frame:

```text
seen_mask[codebook][sampled_code] = true
```

Reset `seen_mask` cho mỗi synthesis segment.

---

# 24. ZeroTTS end-of-audio semantics

Khi `<eoa>` xảy ra, frame code cùng bước đã tồn tại.

Không drop ngay frame đó.

Config:

```text
eoa_extra_frames = 1
```

Flow:

```text
is_eoa first seen
   ↓
start tail counter
   ↓
keep generated audio tail
   ↓
stop after tail drained
```

Nếu drop ngay khi `<eoa>`:

```text
final phone release có thể bị clip
```

---

# 25. ZeroTTS streaming codec

Codec input khác dtype với TTS graph.

TTS graph code:

```text
int64
```

Codec code:

```text
int32
```

Axis:

TTS produces:

```text
(B, K, T)
```

codec wants:

```text
(B, T, K)
```

Phải transpose.

## 25.1 Codec state initialization

`decode_step` có streaming state.

Đặc biệt:

```text
cached position ring
```

phải init:

```text
-1
```

không phải `0`.

`0` là valid position.

Đây là regression test bắt buộc.

## 25.2 Codec output

Codec upstream trả stereo-like tensor rồi upstream runtime average thành mono.

Provider output:

```rust
TtsPcmChunk {
    sample_rate_hz: 48_000,
    channels: 1,
    samples: mono_f32,
}
```

---

# 26. ZeroTTS worker streaming

Pseudo-code:

```rust
loop {
    if cancelled {
        return Err(TtsError::Cancelled);
    }

    let frame_codes = generator.next_frame()?;

    if frame_codes.is_none() {
        break;
    }

    codec_codes.push(frame_codes.unwrap());

    if codec_codes.len() >= codec_batch_frames {
        let pcm = codec.decode_step(&codec_codes)?;
        codec_codes.clear();

        output_tx
            .blocking_send(TtsWorkerEvent::Pcm(pcm))
            .map_err(|_| TtsError::ConsumerClosed)?;
    }
}
```

`codec_batch_frames` cần benchmark.

Default latency thấp:

```text
1–2 frames
```

vì mỗi ZeroTTS frame ~80 ms.

Không gom vài giây audio trước decode.

---

# 27. TTS text segmentation

LLM stream không nên đẩy từng token vào ZeroTTS.

Pipeline:

```text
LLM tokens
   ↓
sentence segmenter
   ↓
TTS segment
```

Một segment nên đủ tự nhiên nhưng không quá dài.

ZeroTTS upstream khuyến nghị segment long-form input.

Project có thể giữ sentence-oriented segmentation hiện tại:

```text
sentence terminator
OR
safe max chars
OR
latency flush threshold
```

Không đưa paragraph rất dài vào một synthesis stream.

`SpeechOutput` giữ `ordinal` cho segment.

---

# 28. Cancellation

Provider phải support cooperative cancellation.

## 28.1 VAD

Session disconnect:

```text
drop VadSession
```

Không có output async sau drop.

## 28.2 ASR

Abort/barge-in:

```text
generation++
cancel ASR worker session
drop OnlineStream
ignore stale worker event by generation gate
```

## 28.3 TTS

`TtsStream::cancel()`:

```text
set cancellation flag
stop generation loop ASAP
do not emit more PCM
drop per-stream KV/cache
```

`SpeechOutput` không chờ synthesize cả sentence sau cancel.

## 28.4 Generation tag

Mọi worker event phải có:

```rust
generation: GenerationId
```

Actor drop stale event.

---

# 29. Bounded queues

Đề xuất queue:

```text
WS ingress                  bounded
decoded PCM -> VAD worker   bounded
PCM -> ASR worker           bounded
LLM -> SpeechOutput         bounded
TTS PCM -> SpeechOutput     bounded
outbound Opus               bounded
```

## 29.1 Overload policy

### VAD PCM full

Audio realtime không thể backpressure microphone vô hạn.

Nếu queue full:

```text
drop current ingress audio frame
telemetry++
```

Tùy severity có thể abort capture nếu continuity không còn đáng tin.

### ASR PCM full

Không nên drop random PCM trong một active utterance mà vẫn coi transcript hợp lệ.

Nếu ASR worker không theo kịp:

```text
fail current recognition
cancel ASR session
return controlled turn failure
```

### TTS PCM full

Backpressure producer.

Không drop synthesized PCM.

---

# 30. Threading và CPU budget

Không để mỗi session tự spawn số inference threads tối đa.

Ví dụ sai:

```text
4 sessions
x Zipformer 4 threads
x ZeroTTS 8 threads
```

sẽ oversubscribe CPU.

Startup config nên có:

```toml
[limits]
max_asr_streams = 2
tts_concurrency = 1
```

Model thread count và request concurrency là hai biến khác nhau.

Baseline CPU:

```text
Silero:    1 inference thread
Zipformer: 2 inference threads
ZeroTTS:   4 inference threads
Codec:     2 inference threads
TTS concurrency: 1
```

Sau đó benchmark trên hardware thật.

---

# 31. ONNX Runtime linking strategy

Project sẽ dùng:

```text
sherpa-onnx
+
ort
```

nếu Zipformer dùng sherpa còn ZeroTTS dùng native Rust ONNX.

Điều này cần quản lý ONNX Runtime cẩn thận.

## 31.1 Baseline dễ triển khai

- `sherpa-onnx` theo build mode đã pin;
- `ort` dùng dynamic loading;
- log version của cả hai runtime khi startup;
- benchmark RSS.

## 31.2 Production tối ưu

Nếu muốn một shared ONNX Runtime:

- build/pin sherpa-onnx phù hợp shared ORT;
- `ort` load cùng compatible runtime;
- CI chạy smoke test trên Linux target thực tế.

Không giả định hai wrapper khác nhau luôn ABI-compatible chỉ vì đều gọi “ONNX Runtime”.

## 31.3 Cargo version pin

Tại thời điểm tài liệu này:

```text
sherpa-onnx Rust docs: 1.13.8
ort docs:               2.0.0-rc.13
```

Pin version trong workspace/Cargo.lock.

Không dùng `*`.

Ví dụ khởi đầu:

```toml
[workspace.dependencies]
sherpa-onnx = "=1.13.8"
ort = { version = "=2.0.0-rc.13", default-features = false, features = ["load-dynamic"] }
ndarray = "0.16"
tokenizers = "0.22"
unicode-normalization = "0.1"
```

Trước khi merge production, chạy `cargo tree` để kiểm tra native dependencies.

---

# 32. Provider errors

Không expose raw FFI error string làm protocol response.

## 32.1 Unified load error

```rust
#[derive(Debug, thiserror::Error)]
pub enum ProviderLoadError {
    #[error("unknown provider adapter: {0}")]
    UnknownAdapter(String),

    #[error("model artifact missing: {0}")]
    MissingArtifact(PathBuf),

    #[error("invalid provider config: {0}")]
    InvalidConfig(String),

    #[error("runtime initialization failed: {0}")]
    Runtime(String),

    #[error("model/tokenizer mismatch: {0}")]
    ModelMismatch(String),
}
```

## 32.2 Runtime errors

```rust
pub enum VadError {
    InvalidAudio,
    Runtime(String),
}

pub enum AsrError {
    InvalidAudio,
    Runtime(String),
    Cancelled,
    Timeout,
}

pub enum TtsError {
    InvalidText,
    VoiceNotFound,
    ModelMismatch,
    Runtime(String),
    EmptyAudio,
    Cancelled,
    Timeout,
}
```

Telemetry có provider name và stable error code.

Không log:

```text
raw audio
transcript
TTS text
voice latent
```

nếu policy project cấm nội dung speech trong telemetry.

---

# 33. Provider worker event model

```rust
pub enum VadWorkerEvent {
    Probability {
        session_id: SessionId,
        start_sample: u64,
        probability: f32,
    },
    Failed {
        session_id: SessionId,
        error: VadError,
    },
}

pub enum AsrWorkerEvent {
    Partial {
        generation: GenerationId,
        text: String,
    },
    Final {
        generation: GenerationId,
        result: AsrResult,
    },
    Failed {
        generation: GenerationId,
        error: AsrError,
    },
}

pub enum TtsWorkerEvent {
    Pcm {
        generation: GenerationId,
        ordinal: u32,
        chunk: TtsPcmChunk,
    },
    SegmentFinished {
        generation: GenerationId,
        ordinal: u32,
    },
    Failed {
        generation: GenerationId,
        error: TtsError,
    },
}
```

Worker event không gửi trực tiếp ra WebSocket.

---

# 34. `SessionActor` integration

Actor chỉ biết provider-facing application events.

Ví dụ:

```rust
pub enum SessionEvent {
    ClientMessage(ClientMessage),
    ClientAudio(bytes::Bytes),

    Vad(VadEvent),

    Turn {
        generation: GenerationId,
        event: TurnEvent,
    },

    Disconnected,
}

pub enum TurnEvent {
    AsrPartial(AsrPartial),
    AsrFinal(AsrResult),
    Llm(LlmEvent),
    SpeechOutput(SpeechOutputEvent),

    ProviderError {
        source: ProviderKind,
        error: ProviderError,
    },
}
```

`AsrPartial`:

- có thể gửi STT display;
- không commit dialogue;
- không start LLM.

`AsrFinal`:

- trim;
- rỗng → `CompletedSilent`;
- non-empty → commit user message rồi start LLM.

---

# 35. Model artifact management

Không commit large weights vào normal Git history.

```text
models/
├── manifest.toml
├── vad/
├── asr/
└── tts/
```

`.gitignore`:

```gitignore
/models/**
!/models/manifest.toml
```

## 35.1 Manifest

```toml
[vad.silero]
source = "https://github.com/snakers4/silero-vad"
artifact = "silero_vad.onnx"
revision = "<pinned commit/version>"
sha256 = "<sha256>"

[asr.zipformer_vi]
source = "hynt/Zipformer-30M-RNNT-Streaming-6000h"
revision = "<pinned HF revision>"
variant = "chunk-32-left-128"

[tts.zerotts]
source = "zeroweight-ai/ZeroTTS"
revision = "<pinned revision>"
```

`fetch-models` script phải:

1. download;
2. verify SHA256;
3. normalize filenames;
4. write no secret;
5. fail on partial download.

Production process không download model giữa request.

---

# 36. Model license gate

License phải là deployment gate.

## Silero VAD

Current upstream project: MIT.

## ZeroTTS

Current upstream code/weights: MIT; bundled MOSS codec decoder có Apache-2.0 notice theo upstream.

## Zipformer Vietnamese model

`hynt/Zipformer-30M-RNNT-Streaming-6000h` hiện công bố:

```text
CC-BY-NC-ND-4.0
```

Đây là điểm cần đặc biệt lưu ý.

Không tự giả định model phù hợp commercial deployment.

Không redistribute transformed/quantized artifacts trước khi review license.

Provider abstraction giúp thay model ASR khác nếu licensing không phù hợp deployment.

---

# 37. Tests bắt buộc: provider contract

Mỗi adapter phải chạy cùng một contract test suite.

## 37.1 VAD contract tests

```text
vad_silence_does_not_start_speech
vad_speech_crosses_threshold
vad_state_is_per_stream
vad_reset_clears_state
vad_accepts_arbitrary_pcm_chunk_boundaries
vad_60ms_transport_frames_are_rechunked_to_model_frames
vad_invalid_sample_rate_fails_cleanly
```

## 37.2 ASR contract tests

```text
asr_open_push_finish
asr_partial_is_optional_but_monotonic_enough_for_display
asr_final_is_emitted_once
asr_cancel_stops_output
asr_empty_audio_does_not_panic
asr_stream_state_is_not_shared
asr_queue_overload_is_controlled_failure
```

Zipformer-specific:

```text
zipformer_loads_chunk32_artifacts
zipformer_uses_16khz
zipformer_drains_after_input_finished
zipformer_tokens_artifact_is_valid
```

## 37.3 TTS contract tests

```text
tts_non_empty_text_emits_pcm
tts_pcm_declares_real_sample_rate
tts_cancel_stops_future_chunks
tts_unknown_voice_is_controlled_error
tts_empty_audio_is_failure
tts_two_streams_do_not_share_kv_state
```

ZeroTTS-specific:

```text
zerotts_special_token_ids_match
zerotts_position_zero_contract
zerotts_audio_frame_position_is_v_plus_1_plus_t
zerotts_seen_mask_resets_per_segment
zerotts_codec_uses_int32_codes
zerotts_codec_axis_is_b_t_k
zerotts_codec_position_cache_initializes_minus_one
zerotts_eoa_tail_is_kept
zerotts_voice_query_count_matches_model
```

---

# 38. Golden/parity tests cho ZeroTTS Rust

Port model không nên chỉ test “có audio”.

Cần parity fixture.

Upstream runtime cho phép deterministic random draws.

Tạo fixture:

```text
text
voice latent
seed/random draws
expected first N audio-code frames
```

Rust test:

```text
same text
same tokenizer ids
same voice
same random draws
   ↓
compare sampled codes
```

Ưu tiên compare:

```text
text token ids
first global hidden checksum
first N frame codes
codec first chunk sample count
```

Không cần bit-exact waveform trên mọi execution provider, nhưng code/frame parity CPU là gate tốt.

---

# 39. Latency metrics

Provider abstraction không được làm mất observability.

Metrics gợi ý:

```text
vad_inference_ms
vad_speech_start_latency_ms

asr_stream_open_ms
asr_decode_ms
asr_partial_latency_ms
asr_final_after_speech_end_ms

tts_load_ms
tts_ttfa_ms
tts_frame_generation_ms
tts_codec_decode_ms
tts_rtf

provider_queue_depth
provider_queue_full_total
provider_error_total{provider,code}
```

Không dùng transcript/text làm metric label.

---

# 40. Warmup

## Silero

Một zero frame qua model.

Reset recurrent state sau warmup.

## Zipformer

Create recognizer + test stream với short silence.

Không giữ stream warmup làm real session.

## ZeroTTS

Upstream warmup logic chạy qua:

```text
text_encoder
prefix_step
local_frame_decode
prefix frame step
codec
```

Warmup trước bind server để first user không chịu allocator/thread-pool cold start.

Warmup failure → fail startup.

---

# 41. Resampling

## Input

Canonical uplink đã 16 kHz.

Silero + Zipformer cũng 16 kHz.

Do đó:

```text
không resample input trong default stack
```

Nếu ASR model tương lai cần 8/48 kHz:

```text
ASR adapter capabilities
+
worker-side input normalizer
```

Không thay wire protocol.

## Output

ZeroTTS output:

```text
48 kHz mono f32
```

Wire:

```text
24 kHz mono Opus
```

`SpeechOutput`:

```text
48k PCM
 ↓
streaming resampler
 ↓
24k PCM
 ↓
1440-sample chunks
 ↓
Opus 60 ms
```

Resampler state phải giữ giữa TTS chunks.

Không resample mỗi chunk độc lập nếu filter cần history.

---

# 42. Manual listen mode

Manual capture và VAD mode phải tách semantics.

## Auto/VAD mode

```text
PCM → VAD → SpeechStart/SpeechEnd
```

## Manual mode

```text
listen:start
  ↓
capture PCM
  ↓
ASR stream open/feed
listen:stop
  ↓
ASR finish
```

VAD provider có thể không tham gia manual mode.

Không biến `listen:detect` thành `listen:start` nếu protocol contract không định nghĩa như vậy.

---

# 43. Barge-in

Khi đang speaking:

```text
microphone vẫn decode
   ↓
Silero confirms new speech
   ↓
generation++
   ↓
GenerationGate updated
   ↓
cancel old TTS
   ↓
drop stale queued audio
   ↓
open new ASR stream
```

Provider không tự tăng generation.

Actor làm việc đó.

---

# 44. Security / robustness

Model file là trusted deployment artifact nhưng config/path vẫn phải validate.

Không:

```rust
unwrap()
expect()
```

trên:

- model result;
- network input;
- malformed voice pack;
- tokenizer;
- provider response.

Có thể dùng `expect()` chỉ cho compile-time invariant hoặc serializer của internal static DTO nếu thực sự impossible.

Voice zip nếu sau này server nhận từ user:

- chống path traversal;
- cap size;
- không extract symlink;
- validate shape;
- không cho override arbitrary filesystem.

---

# 45. Cargo và feature organization

Để binary có thể build nhẹ theo deployment, có thể thêm features.

```toml
[features]
default = [
    "provider-vad-silero",
    "provider-asr-zipformer",
    "provider-tts-zerotts",
]

provider-vad-silero = ["dep:ort"]
provider-asr-zipformer = ["dep:sherpa-onnx"]
provider-tts-zerotts = [
    "dep:ort",
    "dep:tokenizers",
    "dep:ndarray",
    "dep:unicode-normalization",
]
```

Registry dùng `#[cfg(feature = "...")]`.

Config chọn adapter đã compile.

Nếu config yêu cầu adapter bị compile-out:

```text
startup fail:
adapter_not_built
```

Không fallback silent sang model khác.

---

# 46. Implementation order

## PR 1 — Provider foundation

Thêm:

```text
providers/
ProviderSet
registry
errors
capabilities
config adapter_config
contract test harness
```

Chưa cần model thật.

Dùng fake providers trong test.

Exit:

```text
server startup chọn fake adapters bằng config
SessionActor không import model runtime
```

## PR 2 — VAD foundation + Silero

Thêm:

```text
PCM types
Opus decoder
FixedFrameBuffer
VadProvider
Silero adapter
VadSegmenter
pre-roll
```

Exit:

```text
fixture speech -> đúng SpeechStart/SpeechEnd
```

## PR 3 — Streaming ASR + Zipformer

Thêm:

```text
AsrProvider/AsrSession
ASR worker
Zipformer sherpa adapter
partial/final events
ASR stream concurrency
```

Exit:

```text
ASR chạy đồng thời lúc user đang nói
final tới gần SpeechEnd
```

## PR 4 — TTS provider foundation

Thêm:

```text
TtsProvider/TtsStream
TTS worker
PCM output contract
SpeechOutput integration
resample
Opus encode/pacing
```

Dùng fake streaming TTS trước.

## PR 5 — ZeroTTS Rust tokenizer + graph runtime

Thêm:

```text
config loader
tokenizer
voice loader
text_encoder
prefix_step
frame decode
parity tests
```

Chưa cần codec streaming nếu muốn PR nhỏ.

## PR 6 — ZeroTTS codec streaming

Thêm:

```text
decode_step state
code transpose/dtype
PCM 48k stream
eoa tail
cancel
```

Exit:

```text
first PCM trước khi toàn utterance synthesize xong
```

## PR 7 — Hardening

```text
warmup
metrics
timeouts
soak
ASR/TTS concurrency
RSS benchmark
model manifest/license docs
```

---

# 47. Thay đổi tài liệu hiện tại của repo

`docs/03-module-contracts.md` đang mô tả batch ASR:

```rust
async fn transcribe(...)
```

Cần thay thành streaming session contract.

Ngoài ra tài liệu hiện ghi VAD local và “chỉ tạo VadProvider khi có ít nhất hai adapter”.

Yêu cầu mới đã xác định VAD cũng phải replaceable, vì vậy cần sửa thành:

```text
VAD là provider seam chính thức giống ASR/TTS.
Segmentation policy vẫn thuộc audio/core.
```

`docs/06-implementation-plan.md` Phase 3 hiện ghi:

```text
ASR trait + first HTTP provider
```

Nên đổi:

```text
VadProvider + Silero local
Streaming AsrProvider + Zipformer local
ASR worker + partial/final
```

Phase 4:

```text
TtsProvider + ZeroTTS local Rust
SpeechOutput PCM normalization/resample/Opus/pacing
```

---

# 48. Default config hoàn chỉnh đề xuất

```toml
[server]
bind = "0.0.0.0:8000"
public_ws_url = "ws://127.0.0.1:8000/voice/v1/"
hello_timeout_ms = 5000
shutdown_grace_ms = 5000

[audio]
input_sample_rate = 16000
output_sample_rate = 24000
channels = 1
frame_ms = 60
max_ws_frame_bytes = 65536

[limits]
max_connections = 4
max_active_turns = 2

max_asr_streams = 2
tts_concurrency = 1

session_event_queue = 64
vad_audio_queue = 64
asr_audio_queue = 64
tts_command_queue = 16
tts_pcm_queue = 16
outbound_control_queue = 32
outbound_audio_queue = 32


[vad]
adapter = "silero_onnx"
min_speech_ms = 180
end_silence_ms = 600
pre_roll_ms = 300
max_utterance_ms = 30000

[vad.adapter_config]
model = "models/vad/silero_vad.onnx"
sample_rate_hz = 16000
window_samples = 512
speech_threshold = 0.50
exit_threshold = 0.35
num_threads = 1


[asr]
adapter = "zipformer_sherpa"
timeout_ms = 15000
partial_emit_interval_ms = 200

[asr.adapter_config]
encoder = "models/asr/zipformer-30m-vi/encoder.onnx"
decoder = "models/asr/zipformer-30m-vi/decoder.onnx"
joiner = "models/asr/zipformer-30m-vi/joiner.onnx"
tokens = "models/asr/zipformer-30m-vi/tokens.txt"

sample_rate_hz = 16000
num_threads = 2
provider = "cpu"
decoding_method = "greedy_search"
enable_internal_endpoint = false


[tts]
adapter = "zerotts_onnx"
timeout_ms = 15000

[tts.adapter_config]
model_dir = "models/tts/zerotts"
voice = "maichi"
execution_provider = "cpu"

intra_op_num_threads = 4
codec_intra_op_num_threads = 2
warmup = true

cfg_scale = 1.0
audio_temperature = 0.8
audio_topk = 25
audio_topp = 0.95
audio_repetition_penalty = 1.2
eoa_extra_frames = 1
max_frames = 1500
```

---

# 49. Acceptance criteria

Provider architecture chỉ được coi là hoàn tất khi đạt đủ:

## Architecture

- `SessionActor` không import sherpa/ort/tokenizers.
- `SpeechOutput` không import ZeroTTS concrete type.
- đổi adapter chỉ cần config + restart.
- mỗi provider có contract tests.

## VAD

- input Opus 60 ms được rechunk đúng cho Silero 512 samples.
- state Silero riêng từng Voice Session.
- pre-roll không mất đầu câu.
- core giữ endpoint semantics.

## ASR

- Zipformer xử lý PCM khi user đang nói.
- có partial result.
- `finish()` drain recognizer.
- stale final không được commit sau cancellation.

## TTS

- ZeroTTS stream PCM trước khi sentence hoàn thành toàn bộ synthesis.
- PCM provider không phụ thuộc Opus.
- resample 48 → 24 kHz streaming.
- first Opus packet chỉ sau `tts:start`.
- cancel không còn stale audio sau `GenerationGate`.

## Reliability

- mọi queue bounded.
- không block Tokio runtime.
- model load một lần startup.
- warmup trước nhận traffic.
- no network model download during a Voice Session.

---

# 50. Nguồn kỹ thuật tham khảo

## Project

- `hailp-vn38/ai-agent-voice`
- `docs/01-system-architecture.md`
- `docs/02-codebase.md`
- `docs/03-module-contracts.md`
- `docs/04-configuration.md`
- `docs/06-implementation-plan.md`

## Silero VAD

- Repository: https://github.com/snakers4/silero-vad
- ONNX usage / model state: https://github.com/snakers4/silero-vad/blob/master/src/silero_vad/utils_vad.py
- sherpa-onnx Silero docs: https://k2-fsa.github.io/sherpa/onnx/vad/silero-vad.html

Các điểm đã kiểm tra khi viết tài liệu:

```text
16 kHz frame = 512 samples = 32 ms
8 kHz frame  = 256 samples = 32 ms
Silero VAD license = MIT
```

## sherpa-onnx Rust

- Rust crate: https://docs.rs/sherpa-onnx/
- Streaming ASR: https://k2-fsa.github.io/sherpa/onnx/c-api/html/online_asr.html

Tại thời điểm tài liệu:

```text
sherpa-onnx crate docs version: 1.13.8
```

Rust API hiện có:

```text
OnlineRecognizer
OnlineRecognizerConfig
OnlineStream
VoiceActivityDetector
SileroVadModelConfig
```

## Zipformer Vietnamese Streaming

- Model: https://huggingface.co/hynt/Zipformer-30M-RNNT-Streaming-6000h
- Model card: https://huggingface.co/hynt/Zipformer-30M-RNNT-Streaming-6000h/blob/main/README.md

Đã kiểm tra:

```text
~30M parameters
Vietnamese
RNNT
chunk 16 / 32 / 64
streaming
ONNX artifacts
license CC-BY-NC-ND-4.0
```

## ZeroTTS

- Repository: https://github.com/zeroweight-ai/ZeroTTS
- Runtime contract: https://github.com/zeroweight-ai/ZeroTTS/blob/main/docs/RUNTIME.md
- Python reference runtime:
  - `src/zerotts/synthesizer.py`
  - `src/zerotts/codec.py`
  - `src/zerotts/tokenizer.py`
  - `src/zerotts/voices.py`

Đã kiểm tra:

```text
inference không cần PyTorch
ONNX Runtime + tokenizer
PCM output 48 kHz
model frame rate 12.5 Hz
streaming codec decode_step
codebook count hiện tại K=16
default eoa_extra_frames=1
ZeroTTS code/weights MIT
bundled MOSS codec decoder Apache-2.0
```

## Rust ONNX Runtime

- `ort`: https://docs.rs/ort/latest/ort/

Tại thời điểm tài liệu:

```text
ort docs version: 2.0.0-rc.13
```

Khuyến nghị xem xét `load-dynamic` để kiểm soát native ONNX Runtime deployment và giảm rủi ro shared-library conflict.

---

# 51. Quyết định mặc định cuối cùng

```text
VAD provider:
  silero_onnx
  Rust + ort
  16 kHz / 512 samples
  provider trả probability
  core VadSegmenter quyết định speech boundary

ASR provider:
  zipformer_sherpa
  Rust + sherpa-onnx OnlineRecognizer
  hynt/Zipformer-30M-RNNT-Streaming-6000h
  chunk-32-left-128
  streaming partial + final

TTS provider:
  zerotts_onnx
  native Rust port của ZeroTTS ONNX runtime
  tokenizers + ort
  streaming codec decode_step
  PCM 48 kHz f32
  SpeechOutput resample 48 → 24 kHz → Opus 60 ms
```

Điểm quan trọng nhất:

```text
Model-specific runtime không được trở thành application architecture.
```

Silero, Zipformer và ZeroTTS chỉ là ba adapter mặc định đầu tiên.

Core phải vẫn hoạt động khi sau này thay bằng:

```text
TenVAD
Whisper / Paraformer / model ASR khác
Piper / Kokoro / model TTS khác
```

mà không sửa `SessionActor`, dialogue flow hoặc WebSocket protocol.
