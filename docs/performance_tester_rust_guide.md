# Hướng dẫn xây dựng `performance_tester` cho `ai-agent-voice`

> Tài liệu thiết kế và triển khai benchmark native Rust cho VAD / ASR / LLM / TTS, dựa trên ý tưởng của `xiaozhi-esp32-server/performance_tester` nhưng bám đúng boundary và runtime hiện tại của `voice-agent-server`.

## 0. Phạm vi và provenance

Tài liệu này được viết theo trạng thái `main` của repository:

- Repository: `https://github.com/hailp-vn38/ai-agent-voice`
- Commit tham chiếu: `6fcbff7e8a1917802adb4dd17fa349a8f51d7679`
- Commit message: `fix(tts): smooth terminal audio and recover pacing after starvation`
- Ngày kiểm tra: `2026-09-24`

Nguồn tham khảo chính:

- `https://github.com/xinnan-tech/xiaozhi-esp32-server/tree/main/main/xiaozhi-server/performance_tester`
- `performance_tester_asr.py`
- `performance_tester_stream_asr.py`
- `performance_tester_llm.py`
- `performance_tester_tts.py`
- `performance_tester_stream_tts.py`
- `performance_tester_vllm.py`

Mục tiêu không phải port các script Python sang Rust theo từng dòng. Mục tiêu là giữ các ý tưởng benchmark hữu ích của Xiaozhi — đặc biệt là **first-output latency**, **total latency**, workload nhiều mẫu và so sánh provider — nhưng đo trực tiếp đúng boundary của server Rust.

---

# 1. Mục tiêu

`performance_tester` phải trả lời bốn nhóm câu hỏi khác nhau.

## 1.1. Provider benchmark

Đo bản thân provider/model:

```text
VAD provider
ASR provider
LLM provider
TTS provider
```

Không sở hữu:

```text
Voice Session
SessionActor
WebSocket
outbound queue
audio pacing
client playback
MCP
```

Đây là lớp dùng để trả lời:

> Model/provider này xử lý nhanh đến đâu trên hardware hiện tại?

---

## 1.2. TTS delivery benchmark

Riêng TTS cần thêm một boundary production-facing:

```text
text
  ↓
TTS provider
  ↓
PCM F32 mono 48 kHz
  ↓
production downlink conversion
  ↓
PCM16 mono 24 kHz / 1,440 samples
  ↓
Opus
  ↓
packet ready
```

Đây là lớp trả lời:

> Từ text đến canonical Opus packet mà server có thể gửi cho client mất bao lâu?

Không đo pacing 60 ms hay WebSocket I/O.

---

## 1.3. Worker runtime benchmark

Đo capacity/concurrency của:

```text
VadWorkerRuntime
AsrWorkerRuntime
LlmRuntime
TtsWorkerRuntime
```

Đây là lớp trả lời:

> Với N tác vụ đồng thời, runtime thật của server chịu tải ra sao?

---

## 1.4. Protocol E2E load test

Chạy bằng Reference Client qua WebSocket thật:

```text
Reference Client
  ↓
WebSocket
  ↓
SessionActor
  ↓
VAD → ASR → LLM → TTS
  ↓
Opus downlink
```

Đây là lớp trả lời:

> Latency mà một Voice Protocol Client thực sự quan sát được là bao nhiêu?

**Không trộn kết quả E2E với Provider Benchmark.**

---

# 2. Những gì học từ `xiaozhi-esp32-server/performance_tester`

Xiaozhi hiện tách các phép đo:

| Tester | Metric chính |
|---|---|
| ASR non-stream | tổng thời gian nhận dạng |
| ASR stream | response đầu tiên |
| LLM | TTFT và tổng response |
| TTS non-stream | tổng thời gian synthesize |
| TTS stream | first audio chunk |
| vLLM | performance riêng của LLM backend |

Điểm nên giữ:

1. Đo first output riêng với total latency.
2. Chạy workload nhiều lần.
3. Ghi nhận failure/timeout thay vì crash toàn bộ process.
4. Cho phép so sánh provider.
5. Có report dễ đọc từ terminal.

Điểm **không nên copy nguyên**:

1. Không dùng Python/threadpool làm lớp trung gian cho local model Rust.
2. Không tính WebSocket handshake/network vào ZeroTTS local provider latency.
3. Không chỉ dùng average.
4. Không tự loại outlier bằng quy tắc ad-hoc.
5. Không hard-code workload mà không version.
6. Không để lỗi config ASR chặn benchmark TTS.
7. Không xây một đường resample/Opus khác production chỉ dành cho benchmark.

---

# 3. Boundary hiện tại của server Rust

## 3.1. VAD

Provider boundary:

```rust
pub trait VadProvider: Send + Sync {
    fn open(&self) -> Result<Box<dyn VadSession>, VadError>;
    fn adapter(&self) -> &'static str;
}

pub trait VadSession: Send {
    fn push(&mut self, input: VadInput) -> Result<VadProbability, VadError>;
    fn reset(&mut self) -> Result<(), VadError>;
    fn close(&mut self) -> Result<(), VadError>;
}
```

Provider input chuẩn:

```text
512 samples
16 kHz
mono
32 ms audio
```

Production worker nhận uplink 960 samples rồi `VadRechunker` chia thành 512 samples.

Vì vậy:

- **provider benchmark** dùng 512-sample input;
- **runtime benchmark** dùng canonical 960-sample uplink input để đo cả rechunking.

---

## 3.2. ASR

Boundary:

```rust
pub trait AsrProvider: Send + Sync {
    fn open(&self) -> Result<Box<dyn AsrSession>, AsrError>;
}

pub trait AsrSession: Send {
    fn push_pcm(&mut self, pcm: &PcmF32Mono) -> Result<Vec<AsrEvent>, AsrError>;
    fn finish(&mut self) -> Result<AsrResult, AsrError>;
    fn cancel(&mut self);
}
```

Zipformer hiện có:

```rust
AsrEvent::Partial(String)
```

và `finish()` tự thêm final padding 0.66 s trước `input_finished()`.

Benchmark phải dùng chính `AsrSession::finish()`, không tự viết lại finalization.

Lưu ý: `AsrWorkerRuntime` hiện không route partial ASR ra SessionActor; `first_partial_ms` vì vậy là **provider metric**, không phải user-visible server latency.

---

## 3.3. LLM

Boundary:

```rust
#[async_trait::async_trait]
pub trait LlmProvider: Send + Sync {
    async fn stream(
        &self,
        request: LlmRequest,
    ) -> Result<LlmEventStream, LlmError>;
}
```

Event:

```rust
LlmEvent::TextDelta(String)
LlmEvent::ToolCall(ToolCall)
LlmEvent::Finished
```

Có thể đo trực tiếp:

```text
request start → first non-empty TextDelta = TTFT
request start → Finished              = total latency
```

---

## 3.4. TTS

Boundary hiện tại:

```rust
pub trait TtsProvider: Send + Sync {
    fn synthesize(...);
    fn synthesize_stream(...);
    fn open_stream(...);
    fn open_worker(...) -> Result<Box<dyn TtsWorker>, TtsError>;
}
```

Production TTS dùng long-lived worker:

```rust
pub trait TtsWorker: Send {
    fn synthesize(
        &mut self,
        text: &str,
        cancelled: &AtomicBool,
        on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
    ) -> Result<(), TtsError>;

    fn reset(&mut self) -> Result<(), TtsError>;
}
```

Do đó steady-state benchmark ZeroTTS phải ưu tiên:

```text
provider build
  ↓
open_worker() một lần
  ↓
qualification/warmup
  ↓
N measured syntheses trên cùng worker
```

Không mở native session mới cho từng câu.

---

# 4. Kiến trúc tổng thể đề xuất

```text
                         performance-tester
                                │
          ┌─────────────────────┼─────────────────────┐
          │                     │                     │
       provider              runtime                 e2e
          │                     │                     │
    ┌─────┼─────┐        Worker Runtimes      Reference Client
    │     │     │
   VAD   ASR   LLM
                \
                 TTS
                  │
          ┌───────┴────────┐
          │                │
       provider         delivery
          │                │
       PCM 48k          PCM 48k
                           │
                    production converter
                           │
                     Opus packet ready
```

Khuyến nghị triển khai `provider` và `delivery` trước. `runtime` và `e2e` là phase tiếp theo.

---

# 5. Cấu trúc thư mục

Đề xuất:

```text
crates/voice-agent-server/
├── src/
│   ├── performance_tester/
│   │   ├── mod.rs
│   │   ├── cli.rs
│   │   ├── error.rs
│   │   ├── loader.rs
│   │   ├── report.rs
│   │   ├── stats.rs
│   │   ├── workload.rs
│   │   │
│   │   ├── provider/
│   │   │   ├── mod.rs
│   │   │   ├── vad.rs
│   │   │   ├── asr.rs
│   │   │   ├── llm.rs
│   │   │   └── tts.rs
│   │   │
│   │   ├── delivery/
│   │   │   ├── mod.rs
│   │   │   └── tts.rs
│   │   │
│   │   └── runtime/
│   │       ├── mod.rs
│   │       ├── vad.rs
│   │       ├── asr.rs
│   │       ├── llm.rs
│   │       └── tts.rs
│   │
│   └── bin/
│       └── performance-tester.rs
│
└── tests/
    └── performance_tester_contract.rs

benchmarks/
└── performance_tester/
    ├── workloads/
    │   ├── vad-v1.json
    │   ├── asr-v1.json
    │   ├── llm-v1.json
    │   └── tts-v1.json
    │
    ├── assets/
    │   ├── asr/
    │   │   ├── vi-short.wav
    │   │   ├── vi-medium.wav
    │   │   └── vi-long.wav
    │   └── vad/
    │       └── speech-silence-16k.wav
    │
    └── reports/
        └── .gitkeep
```

`reports/` có thể nằm ngoài Git nếu muốn; workload và fixture phải được version control.

---

# 6. Cargo thay đổi

Workspace đã có:

```toml
clap = { version = "4.6", features = ["derive"] }
```

Nhưng `voice-agent-server` hiện chưa khai báo `clap`.

Thêm vào:

```toml
# crates/voice-agent-server/Cargo.toml

[dependencies]
clap.workspace = true
```

Các dependency cần thiết khác đã tồn tại:

```text
anyhow
futures-util
hound
serde
serde_json
sha2
tokio
tokio-util
```

Không cần Criterion cho harness chính.

Criterion chỉ phù hợp nếu sau này muốn microbenchmark:

```text
DownlinkResampler
Opus encode
text sanitization
sentence segmentation
```

---

# 7. Binary entrypoint

`src/bin/performance-tester.rs` phải mỏng.

Ví dụ:

```rust
use clap::Parser;
use voice_agent_server::performance_tester::{PerformanceTesterCli, run};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    run(PerformanceTesterCli::parse()).await
}
```

Thêm vào `lib.rs`:

```rust
pub mod performance_tester;
```

Tất cả logic benchmark nằm trong library module để có thể truy cập các API `pub(crate)` của server mà không phải public hóa provider internals chỉ vì CLI.

---

# 8. CLI contract

Đề xuất CLI:

```text
performance-tester
  provider
    vad
    asr
    llm
    tts

  runtime
    vad
    asr
    llm
    tts
```

TTS provider có mode riêng:

```text
--mode provider
--mode delivery
```

Ví dụ:

```bash
cargo run --release -p voice-agent-server \
  --bin performance-tester -- \
  provider tts \
  --config config.toml \
  --workload benchmarks/performance_tester/workloads/tts-v1.json \
  --mode provider \
  --warmup 3 \
  --iterations 20 \
  --output benchmarks/performance_tester/reports/tts-provider.json
```

Delivery:

```bash
cargo run --release -p voice-agent-server \
  --bin performance-tester -- \
  provider tts \
  --config config.toml \
  --workload benchmarks/performance_tester/workloads/tts-v1.json \
  --mode delivery \
  --warmup 3 \
  --iterations 20 \
  --output benchmarks/performance_tester/reports/tts-delivery.json
```

ASR:

```bash
cargo run --release -p voice-agent-server \
  --bin performance-tester -- \
  provider asr \
  --config config.toml \
  --workload benchmarks/performance_tester/workloads/asr-v1.json \
  --feed burst \
  --warmup 2 \
  --iterations 10 \
  --output benchmarks/performance_tester/reports/asr.json
```

LLM:

```bash
cargo run --release -p voice-agent-server \
  --bin performance-tester -- \
  provider llm \
  --config config.toml \
  --workload benchmarks/performance_tester/workloads/llm-v1.json \
  --warmup 1 \
  --iterations 10 \
  --output benchmarks/performance_tester/reports/llm.json
```

VAD:

```bash
cargo run --release -p voice-agent-server \
  --bin performance-tester -- \
  provider vad \
  --config config.toml \
  --workload benchmarks/performance_tester/workloads/vad-v1.json \
  --warmup 5 \
  --iterations 100 \
  --output benchmarks/performance_tester/reports/vad.json
```

---

# 9. CLI types

Ví dụ:

```rust
#[derive(clap::Parser)]
pub struct PerformanceTesterCli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(clap::Subcommand)]
pub enum Command {
    Provider(ProviderCommand),
    Runtime(RuntimeCommand),
}

#[derive(clap::Args)]
pub struct ProviderCommand {
    #[command(subcommand)]
    pub target: ProviderTarget,
}

#[derive(clap::Subcommand)]
pub enum ProviderTarget {
    Vad(VadArgs),
    Asr(AsrArgs),
    Llm(LlmArgs),
    Tts(TtsArgs),
}

#[derive(clap::ValueEnum, Clone, Copy)]
pub enum TtsMode {
    Provider,
    Delivery,
}

#[derive(clap::ValueEnum, Clone, Copy)]
pub enum AsrFeedMode {
    Burst,
    Realtime,
}
```

Các option chung:

```text
--config <path>
--workload <path>
--warmup <N>
--iterations <N>
--output <path>
--label <string>          optional
--fail-on-error           default true
```

Không cho phép:

```text
warmup = 0         nếu benchmark cần qualification
iterations = 0
```

Có thể cho `--warmup 0` về sau, nhưng baseline đầu tiên nên buộc ít nhất một qualification pass.

---

# 10. Quy tắc timing

Luôn dùng:

```rust
std::time::Instant
```

Không dùng:

```text
SystemTime cho latency
wall-clock date
DateTime subtraction
```

`SystemTime` chỉ dùng để ghi timestamp report.

---

## 10.1. Normative contract đã chốt cho TTS benchmark v1

Các quy tắc dưới đây là contract của `performance-tester`, không phải lựa chọn implementation có thể thay đổi âm thầm:

```text
tts-v1
= independent utterances
= production provider
= reset giữa mọi sample
= provider mode kết thúc ở PCM provider-facing
= delivery mode dùng production deterministic conversion
= fade-out 480 downlink samples ở 24 kHz = 20 ms
= tail zero-pad tới 1,440 samples
= exact production Opus settings
= không pacing / Voice Session / WebSocket
= không có metric bị gọi sai là provider-exclusive
```

`tts-continuity-v1` hoặc `--mode continuity` là workload/mode riêng nếu cần đo nhiều
`Speech Segment` liên tục trong cùng một `Generated Assistant Response`. Không trộn continuity
vào `tts-v1`.

Trong delivery mode, conversion chạy đồng bộ trong callback của `worker.synthesize()`. Vì vậy
elapsed quanh lời gọi đó là `synthesis_with_delivery_ms`, không phải provider-exclusive time.
Schema v1 không có `provider_exclusive_ms`; không được suy ra nó bằng cách trừ encode time khi
chưa có instrumentation đủ chặt.

---

# 11. Cold / qualification / steady-state

Phải tách ba phase.

```text
config parse/resolve
      │
      ▼
model preparation
      │
      ▼
provider build + production warmup
      │
      ▼
worker open nếu target dùng worker
      │
      ▼
qualification/warmup iterations
      │
      ├── validate output
      └── discard latency
      │
      ▼
========== STEADY BOUNDARY ==========
      │
      ▼
measured iterations
```

## 11.1. Không gộp init với steady

Report riêng:

```text
config_load_ms
model_preparation_ms
provider_build_warmup_ms
worker_open_ms
qualification_ms
```

Không tạo một field mơ hồ:

```text
init_ms
```

nếu có thể tách được các boundary trên.

---

## 11.2. Model acquisition

Nếu Model Preparation phải download/transform artifact trong run đó:

```json
{
  "model_preparation": {
    "artifact_acquisition_occurred": true
  }
}
```

Run như vậy vẫn hữu ích để quan sát startup nhưng **không được so trực tiếp cold-start với run warm-cache**.

Steady-state vẫn có thể dùng nếu provider và workload sau đó giống nhau.

---

# 12. Qualification semantics

Qualification không phải performance sample.

Nó dùng để xác nhận benchmark đang đo output hợp lệ.

## VAD

Mỗi probability:

```text
finite
0.0 <= probability <= 1.0
end_sample = start_sample + 512
timeline liên tục
```

## ASR

Với fixture speech:

```text
finish() thành công
final text non-empty
```

Nếu workload có `expected_text`, có thể kiểm tra normalized exact/contains nhưng không nên biến benchmark thành WER suite.

## LLM

Mặc định text workload phải có:

```text
>= 1 non-empty TextDelta
Finished
```

Không coi `ToolCall` là text TTFT.

Nếu workload dành riêng cho tool call thì phải có workload type riêng.

## TTS provider

Output phải:

```text
sample_rate = 48_000
mono domain type
samples non-empty
all finite
audio duration > 0
```

## TTS delivery

Phải có:

```text
>= 1 Opus packet
mọi packet non-empty
packet size <= configured max
finish() thành công
```

Qualification failure:

1. Ghi failure vào report.
2. Không chạy steady-state.
3. Vẫn flush JSON report nếu có thể.
4. Process exit non-zero.

---

# 13. Không tự loại outlier

Không áp dụng:

```text
mean + 3σ
trim top 5%
drop slow requests
```

Mỗi iteration thành công phải giữ nguyên trong report.

Failure không tham gia percentile latency, nhưng phải tham gia:

```text
attempt_count
success_count
failure_count
success_rate
```

Việc lọc outlier có thể che regression scheduler, GC/native allocator, network jitter hoặc worker contention.

---

# 14. Statistics

Tối thiểu report:

```text
min
mean
p50
p95
p99
max
```

Khuyến nghị percentile deterministic theo nearest-rank.

Pseudo:

```rust
fn percentile_nearest_rank(sorted: &[f64], q: f64) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }

    let rank = (q * sorted.len() as f64).ceil() as usize;
    let index = rank.saturating_sub(1).min(sorted.len() - 1);

    Some(sorted[index])
}
```

Sort bằng:

```rust
f64::total_cmp
```

để tránh undefined ordering.

---

# 15. Workload phải versioned

Không nhúng vài câu test trực tiếp trong code.

Mỗi file:

```json
{
  "schema_version": 1,
  "workload_version": "tts-v1",
  "items": []
}
```

Report phải ghi:

```text
workload_version
workload_sha256
```

`workload_sha256` lấy SHA-256 bytes chính xác của file workload đã đọc.

Như vậy commit A và B chỉ được so khi cùng workload hash/version, trừ khi người dùng cố ý override.

---

# 16. TTS workload

Ví dụ `tts-v1.json`:

```json
{
  "schema_version": 1,
  "workload_version": "tts-v1",
  "items": [
    {
      "id": "vi-short",
      "text": "Xin chào, mình có thể giúp gì cho bạn hôm nay?"
    },
    {
      "id": "vi-medium",
      "text": "Hôm nay chúng ta sẽ kiểm tra tốc độ tổng hợp giọng nói và độ trễ của gói âm thanh đầu tiên."
    },
    {
      "id": "vi-long",
      "text": "Đây là một đoạn văn dài hơn dùng để kiểm tra khả năng tổng hợp liên tục, tốc độ tạo âm thanh, độ ổn định của luồng PCM và chi phí chuyển đổi sang định dạng Opus mà thiết bị sử dụng."
    }
  ]
}
```

Không thay đổi text của `tts-v1` sau khi đã dùng làm baseline.

Muốn đổi thì tạo:

```text
tts-v2
```

---

# 17. ASR workload

Ví dụ:

```json
{
  "schema_version": 1,
  "workload_version": "asr-v1",
  "items": [
    {
      "id": "vi-short",
      "path": "assets/asr/vi-short.wav",
      "expected_text": "xin chào"
    },
    {
      "id": "vi-medium",
      "path": "assets/asr/vi-medium.wav"
    }
  ]
}
```

Path resolve tương đối với thư mục workload hoặc một root được quy định rõ; không phụ thuộc current working directory một cách ngầm định.

Fixture ASR baseline:

```text
WAV PCM
mono
16-bit
16 kHz
```

Loader phải reject file khác profile thay vì resample âm thầm trong provider benchmark.

Nếu muốn benchmark resampling đầu vào thì tạo target riêng, không trộn vào ASR provider result.

---

# 18. VAD workload

Ví dụ:

```json
{
  "schema_version": 1,
  "workload_version": "vad-v1",
  "items": [
    {
      "id": "speech-silence",
      "path": "assets/vad/speech-silence-16k.wav"
    }
  ]
}
```

Provider benchmark chia input thành đúng:

```text
512 samples
```

Phần dư cuối không đủ 512:

- mặc định reject fixture nếu muốn strict deterministic workload; hoặc
- drop có khai báo trong report.

Khuyến nghị baseline fixture có số samples chia hết cho 512.

---

# 19. LLM workload

LLM workload phải chứa exact request content để prompt không thay đổi âm thầm khi server persona thay đổi.

Ví dụ:

```json
{
  "schema_version": 1,
  "workload_version": "llm-v1",
  "items": [
    {
      "id": "greeting",
      "messages": [
        {
          "role": "system",
          "content": "Bạn là trợ lý giọng nói. Trả lời ngắn gọn, tự nhiên."
        },
        {
          "role": "user",
          "content": "Xin chào, hôm nay bạn có thể giúp gì cho tôi?"
        }
      ]
    },
    {
      "id": "simple-factual",
      "messages": [
        {
          "role": "system",
          "content": "Bạn là trợ lý giọng nói. Trả lời ngắn gọn, tự nhiên."
        },
        {
          "role": "user",
          "content": "Hãy giải thích ngắn gọn vì sao bầu trời có màu xanh."
        }
      ]
    }
  ]
}
```

Không lấy trực tiếp prompt production theo thời gian nếu mục tiêu là so performance giữa commit.

Có thể thêm workload khác:

```text
llm-production-prompt-v1
```

nếu muốn đo đúng kích thước production prompt.

---

# 20. Target-scoped config validation

Đây là thay đổi cần làm sớm.

Hiện:

```rust
AppConfig::load(...)
```

gọi:

```rust
config.validate()?;
```

và `validate()` kiểm tra toàn bộ:

```text
transport
audio
capacity
workers
providers VAD/ASR/LLM/TTS
speech output
deployment
```

Kết quả:

> Benchmark TTS có thể fail chỉ vì ASR config không hợp lệ.

Điều này sai với boundary Provider Benchmark.

---

# 21. Refactor `AppConfig` loader

Giữ production behavior không đổi:

```rust
pub fn load(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
    let config = Self::parse_and_resolve(path)?;
    config.validate()?;
    Ok(config)
}
```

TOML benchmark vẫn phải deserialize được thành toàn bộ canonical `AppConfig`; scoped validation
chỉ giảm phạm vi **semantic validation**, không tạo `BenchmarkConfig` partial song song.

Thêm parse/resolve dùng chung:

```rust
impl AppConfig {
    pub(crate) fn parse_and_resolve(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let path = path.as_ref();
        let mut config: Self = toml::from_str(&fs::read_to_string(path)?)?;
        config.resolve_agent(path)?;
        Ok(config)
    }
}
```

Thêm benchmark target:

```rust
#[derive(Clone, Copy, Debug)]
pub(crate) enum BenchmarkTarget {
    VadProvider,
    AsrProvider,
    LlmProvider,
    TtsProvider,
    TtsDelivery,
    VadRuntime,
    AsrRuntime,
    LlmRuntime,
    TtsRuntime,
}
```

Thêm:

```rust
pub(crate) fn load_for_benchmark(
    path: impl AsRef<Path>,
    target: BenchmarkTarget,
) -> Result<Self, ConfigError> {
    let config = Self::parse_and_resolve(path)?;
    config.validate_for_benchmark(target)?;
    Ok(config)
}
```

Production `AppConfig::load()` vẫn giữ contract parse + resolve + full validation.

---

# 22. Validation matrix

Khuyến nghị:

| Target | Validate |
|---|---|
| VAD provider | deployment/model root, ONNX runtime, VAD provider options |
| ASR provider | deployment/model root, ASR provider options |
| LLM provider | LLM adapter/base URL/model/timeout |
| TTS provider | deployment/model root, ONNX runtime, TTS provider options |
| TTS delivery | TTS provider + canonical downlink audio constraints |
| VAD runtime | VAD provider + `workers.vad` |
| ASR runtime | ASR provider + `workers.asr` |
| LLM runtime | LLM provider + `limits.llm_concurrency` |
| TTS runtime | TTS provider + `workers.tts` + relevant TTS limits |

Không validate:

```text
MCP
WebSocket bind
unrelated provider
dialogue history
SpeechOutput segmentation
```

trong provider benchmark nếu target không dùng chúng.

---

# 23. Target-scoped provider loader

Hiện `providers::loader::load_local()` luôn:

```text
prepare VAD
prepare ASR
prepare TTS

build VAD
build ASR
build LLM
build TTS
```

Cần thêm loader scoped.

Một lựa chọn:

```rust
pub(crate) struct LoadTimings {
    pub model_preparation: Duration,
    pub provider_build: Duration,
}

pub(crate) struct LoadedProvider<T: ?Sized> {
    pub provider: Arc<T>,
    pub timings: LoadTimings,
}
```

Functions:

```rust
pub(crate) fn load_vad_for_benchmark(...)
    -> Result<LoadedProvider<dyn VadProvider>, ProviderLoadError>;

pub(crate) fn load_asr_for_benchmark(...)
    -> Result<LoadedProvider<dyn AsrProvider>, ProviderLoadError>;

pub(crate) fn load_llm_for_benchmark(...)
    -> Result<LoadedProvider<dyn LlmProvider>, ProviderLoadError>;

pub(crate) fn load_tts_for_benchmark(...)
    -> Result<LoadedProvider<dyn TtsProvider>, ProviderLoadError>;
```

Không cần tạo `ProviderSet` đầy đủ.

---

# 24. Tách Model Preparation timing khỏi provider build

Ví dụ TTS:

```text
start_prepare
   prepare(...)
end_prepare

start_build
   tts_factory.build(...)
end_build
```

Report:

```json
{
  "initialization": {
    "model_preparation_ms": 12.3,
    "provider_build_warmup_ms": 8431.5
  }
}
```

Tên `provider_build_warmup_ms` là có chủ ý vì `ConfiguredZeroTts::load()` hiện chạy startup readiness synthesis và validation trong quá trình build.

Không gọi field đó chỉ là `model_load_ms`.

---

# 25. TTS provider benchmark

## 25.1. Worker boundary

Sau provider load:

```rust
let started = Instant::now();
let mut worker = provider.open_worker()?;
let worker_open_ms = started.elapsed();
```

Nếu `open_worker()` lỗi, benchmark phải fail.

**Không dùng fallback `ProviderWorker`** trong provider benchmark vì fallback sẽ che lỗi native worker.

Production `TtsWorkerRuntime` có fallback để giữ compatibility; benchmark provider cần biết chính native worker có hoạt động hay không.

---

## 25.2. Qualification, warmup và reset

Ví dụ:

```rust
let cancelled = AtomicBool::new(false);

worker.synthesize(
    &item.text,
    &cancelled,
    &mut |pcm| {
        validate_pcm(&pcm)?;
        Ok(())
    },
)?;
```

`tts-v1` coi mọi item là independent utterance. Sau **mọi** warmup item, qualification sample
và measured sample, benchmark phải gọi:

```rust
worker.reset()?;
```

Nếu `reset()` thất bại, state independence đã mất: không lấy sample kế tiếp và fail run thay vì
ghi sample lỗi rồi tiếp tục. Reset không được tính vào latency sample. Production stream semantics
giữ state qua nhiều `Speech Segment` thuộc một `Generated Assistant Response` phải dùng
`tts-continuity-v1` hoặc mode riêng.

---

## 25.3. Metrics

Mỗi measured iteration:

```text
time_to_first_pcm_ms
synthesis_total_ms
pcm_chunks
pcm_samples
audio_duration_ms
rtf
x_realtime
```

Công thức:

```text
audio_duration_ms = pcm_samples / 48000 * 1000

RTF = synthesis_total_ms / audio_duration_ms

x_realtime = audio_duration_ms / synthesis_total_ms
```

Ví dụ:

```text
generated audio = 4.0 s
synthesis       = 1.0 s

RTF            = 0.25
x_realtime     = 4.0
```

---

# 26. First PCM timing

Timer bắt đầu ngay trước:

```rust
worker.synthesize(...)
```

Callback đầu tiên có PCM hợp lệ:

```rust
if first_pcm_at.is_none() {
    first_pcm_at = Some(start.elapsed());
}
```

Không tính:

```text
config parse
model preparation
worker open
qualification
```

vào `time_to_first_pcm_ms`.

---

# 27. TTS delivery benchmark

Đây là phần cần refactor production code trước.

Hiện `SpeechOutput` sở hữu:

```text
DownlinkResampler
DownlinkOpusEncoder
first PCM fade-in
downlink tail
fade-out tail
zero padding
1,440-sample framing
```

Code nằm chủ yếu ở:

```text
session/speech_output/pipeline.rs
```

Không nên instantiate toàn bộ `SpeechOutput` cho benchmark vì sẽ kéo thêm:

```text
SentenceSegmenter
JsonFilter
pending segment queue
worker lease
prebuffer
pacing
playback deadlines
Session-oriented semantics
```

---

# 28. Extract `DownlinkDeliveryEncoder`

Đề xuất file:

```text
src/audio/downlink_delivery.rs
```

API:

```rust
pub struct DownlinkDeliveryEncoder {
    resampler: DownlinkResampler,
    encoder: DownlinkOpusEncoder,
    first_pcm_chunk: bool,
    tail: Vec<i16>,
}

impl DownlinkDeliveryEncoder {
    pub fn new(max_packet_bytes: usize) -> Result<Self, AudioError>;

    pub fn push_provider_pcm(
        &mut self,
        pcm: PcmF32Mono,
    ) -> Result<Vec<OpusPacket>, AudioError>;

    pub fn finish(
        &mut self,
    ) -> Result<Vec<OpusPacket>, AudioError>;
}
```

`SpeechOutput` sau đó giữ:

```rust
delivery: DownlinkDeliveryEncoder
```

thay vì tự giữ:

```text
encoder
downlink_resampler
first_pcm_chunk
downlink_tail
```

---

# 29. `DownlinkDeliveryEncoder` phải preserve production behavior

Không được thay algorithm trong cùng refactor.

Phải giữ:

```text
provider PCM = 48 kHz mono F32
first PCM fade-in = 8 ms
stateful resampler 48 kHz → 24 kHz
float → i16 conversion giống production
frame = 1,440 samples
Opus encoder production options
tail fade-out = 480 downlink samples = 20 ms tại 24 kHz
zero-pad final frame
final Opus packet
```

Opus options hiện nằm trong `DownlinkOpusEncoder::new()`:

```text
24 kHz
mono
Application::Voip
32 kbps
VBR = true
constrained VBR = true
DTX = false
FEC = false
packet loss = 0
complexity = 10
```

Benchmark phải reuse `DownlinkOpusEncoder`; không tạo encoder riêng.

---

# 30. Delivery tail fidelity

Final tail là bắt buộc.

Nếu sau resample còn:

```text
0 < tail.len() < 1,440
```

phải:

```text
fade-out
   ↓
zero-pad đến 1,440
   ↓
Opus encode
```

Không được drop tail.

Nếu drop:

```text
packet_count sai
encoded_duration sai
final_packet_ready sai
runtime fidelity sai
```

Boundary `delivery` kết thúc khi **final canonical Opus packet đã sẵn sàng**, không phải khi chỉ encode xong các full frame.

---

# 31. Delivery dùng cùng PCM stream với provider measurement

Trong một measured TTS synthesis:

```text
TTS worker callback PCM
       │
       ├── update PCM metrics
       │
       └── push chính chunk đó vào DownlinkDeliveryEncoder
```

Không synthesize hai lần:

```text
run A để đo provider
run B để đo delivery
```

trong cùng delivery sample.

Lý do:

- output timing có thể khác;
- model state/cache có thể khác;
- số PCM chunk có thể khác;
- làm kết quả `first_pcm` và `first_opus` không còn cùng một request.

---

# 32. TTS delivery metrics

Mỗi sample:

```text
first_pcm_ms
first_opus_ms
synthesis_with_delivery_ms
delivery_total_ms
pcm_chunks
pcm_samples
audio_duration_ms
opus_packets
opus_bytes
rtf_synthesis_with_delivery
rtf_delivery
```

Định nghĩa:

```text
start
  ↓
worker.synthesize()
  ↓
first PCM callback
  ↓
first full 1,440 downlink frame encoded
  ↓
...
  ↓
worker synthesize returns
  ↓
delivery.finish()
  ↓
final Opus packet ready
```

`synthesis_with_delivery_ms`:

```text
start → worker.synthesize() returns, bao gồm deterministic delivery work chạy đồng bộ trong callback
```

`delivery_total_ms`:

```text
start → delivery.finish() returns
```

Không bao gồm pacing 60 ms.

`provider_exclusive_ms` không thuộc schema v1 và không được suy ra bằng phép trừ encode time.

---

# 33. TTS delivery test equivalence

Sau refactor phải có unit test proving production conversion không đổi.

Ví dụ:

1. Feed cùng deterministic PCM chunks.
2. Old helper/reference và new encoder tạo cùng packet count.
3. Decode Opus packet hoặc so semantic output nếu encoded bytes không được bảo đảm bit-identical trên backend/platform.
4. Verify final packet tồn tại khi tail không đủ 1,440.
5. Verify arbitrary provider chunk boundaries không đổi downlink waveform semantics.
6. Verify first fade-in chỉ áp dụng một lần cho cả delivery generation.
7. Verify fade-out áp dụng ở terminal tail.

Sau khi refactor production `SpeechOutput` phải dùng chính `DownlinkDeliveryEncoder`, nên benchmark và runtime không thể drift.

---

# 34. ASR provider benchmark

## 34.1. Input

Load WAV bằng `hound`.

Reject nếu:

```text
channels != 1
sample_rate != 16000
bits_per_sample != 16
```

Convert:

```rust
f32 = sample as f32 / i16::MAX as f32
```

---

## 34.2. Chunking

Dùng canonical server chunk:

```text
960 samples
60 ms
```

Mặc dù `AsrSession::push_pcm()` có thể nhận chunk lớn hơn, baseline nên mô phỏng transport-to-provider boundary hiện tại.

---

## 34.3. Feed modes

### `burst`

```text
push chunk 1
push chunk 2
...
push chunk N
finish
```

Không sleep.

Dùng để đo compute capacity.

### `realtime`

```text
push 960 samples
sleep đến 60 ms cadence
push 960 samples
...
finish
```

Dùng để đo wall latency khi audio tới giống client thật.

Không trộn hai mode vào cùng percentile.

---

# 35. ASR active compute vs wall time

Trong realtime mode, cần tách:

```text
wall_ms
compute_ms
```

`wall_ms` gồm thời gian chờ audio cadence.

`compute_ms` là tổng:

```text
duration(push_pcm calls)
+ duration(finish)
```

RTF dùng:

```text
compute_ms / audio_duration_ms
```

không dùng realtime `wall_ms`, vì nếu dùng wall time thì RTF gần 1 một cách giả tạo.

---

# 36. ASR metrics

Mỗi sample:

```text
session_open_ms
first_partial_compute_ms
first_partial_wall_ms
finish_ms
compute_total_ms
wall_total_ms
audio_duration_ms
rtf_compute
x_realtime_compute
partial_count
final_text_chars
```

Trong burst mode:

```text
compute và wall gần nhau
```

Trong realtime mode:

```text
wall ≈ audio duration + finalization
compute << wall
```

---

# 37. ASR first partial

First partial là:

```rust
AsrEvent::Partial(text)
```

với:

```text
text.trim().is_empty() == false
```

Không đo first event rỗng.

Lưu ý report metadata:

```json
{
  "notes": [
    "first_partial_ms is provider-level; current AsrWorkerRuntime does not expose partial text to SessionActor"
  ]
}
```

để tránh hiểu nhầm thành client-observed latency.

---

# 38. VAD provider benchmark

Load fixture 16 kHz mono.

Chia thành:

```text
512-sample VadInput
```

Mỗi call:

```rust
let started = Instant::now();
let probability = session.push(input)?;
let elapsed = started.elapsed();
```

Metrics:

```text
session_open_ms
frame_latency_us
p50_us
p95_us
p99_us
frames_per_second
compute_rtf
```

Một frame là:

```text
512 / 16000 = 32 ms
```

RTF:

```text
mean frame compute / 32 ms
```

VAD thường nhanh nên lưu microseconds trong raw sample, có thể render milliseconds ở summary nếu thích.

---

# 39. LLM provider benchmark

## 39.1. TTFT

Start ngay trước:

```rust
provider.stream(request).await
```

TTFT là lúc nhận:

```rust
LlmEvent::TextDelta(text)
```

đầu tiên có:

```text
!text.trim().is_empty()
```

Không dùng:

```text
ToolCall
Finished
empty TextDelta
```

làm TTFT.

---

## 39.2. Total latency

Kết thúc khi:

```rust
LlmEvent::Finished
```

hoặc stream kết thúc theo contract được adapter chuẩn hóa.

Metrics:

```text
ttft_ms
total_ms
text_delta_count
output_chars
tool_call_count
success/failure
```

Hiện provider interface không expose official token usage nên không tự đoán:

```text
tokens_per_second
```

Nếu sau này provider trả usage authoritative thì bổ sung.

---

# 40. Remote LLM benchmark không nên là hard performance gate mặc định

LLM hiện là remote OpenAI-compatible provider.

Latency chịu:

```text
internet
provider queue
region
TLS/session behavior
upstream load
model serving changes
```

Vì vậy:

- lưu report;
- theo dõi trend;
- có thể alert;
- không dùng p95 remote LLM làm CI regression gate cứng giữa hai commit code local trừ khi test environment được kiểm soát.

---

# 41. Report schema

JSON là artifact authoritative.

Terminal table chỉ là presentation.

Ví dụ:

```json
{
  "schema_version": 1,
  "benchmark_kind": "provider",
  "target": "tts",
  "mode": "delivery",

  "repository": {
    "git_sha": "6fcbff7e8a1917802adb4dd17fa349a8f51d7679",
    "dirty": false
  },

  "environment": {
    "os": "linux",
    "arch": "x86_64",
    "available_parallelism": 8,
    "build_profile": "release"
  },

  "provider": {
    "adapter": "zerotts_onnx",
    "model": "zerotts_default",
    "voice": "maichi",
    "num_threads": 4
  },

  "workload": {
    "schema_version": 1,
    "version": "tts-v1",
    "sha256": "..."
  },

  "run": {
    "warmup_iterations": 3,
    "measured_iterations": 20
  },

  "initialization": {
    "config_load_ms": 1.4,
    "model_preparation_ms": 14.7,
    "artifact_acquisition_occurred": false,
    "provider_build_warmup_ms": 8421.2,
    "worker_open_ms": 91.3,
    "qualification_ms": 603.1
  },

  "summary": {
    "attempt_count": 60,
    "success_count": 60,
    "failure_count": 0,
    "first_pcm_ms": {
      "min": 102.0,
      "mean": 114.2,
      "p50": 112.0,
      "p95": 128.0,
      "p99": 132.0,
      "max": 134.0
    },
    "first_opus_ms": {
      "p50": 128.0,
      "p95": 144.0
    },
    "rtf_delivery": {
      "p50": 0.21,
      "p95": 0.24
    }
  },

  "samples": []
}
```

Giữ raw samples để có thể phân tích lại mà không rerun model.

---

# 42. Raw sample schema

Ví dụ TTS:

```json
{
  "iteration": 4,
  "item_id": "vi-medium",
  "success": true,

  "first_pcm_ms": 113.2,
  "first_opus_ms": 129.8,

  "synthesis_with_delivery_ms": 712.0,
  "delivery_total_ms": 715.4,

  "pcm_chunks": 19,
  "pcm_samples": 154320,
  "audio_duration_ms": 3215.0,

  "opus_packets": 54,
  "opus_bytes": 12941,

  "rtf_synthesis_with_delivery": 0.221,
  "rtf_delivery": 0.223
}
```

Failure:

```json
{
  "iteration": 7,
  "item_id": "vi-long",
  "success": false,
  "error_kind": "provider_failure",
  "error_code": "tts_synthesis_failed",
  "message": "TTS synthesis failed"
}
```

Không ghi secret/api key hoặc raw provider error.

---

# 43. Environment fingerprint

Report nên có đủ thông tin để biết hai run có đáng so hay không.

Tối thiểu:

```text
git SHA
dirty state nếu xác định được
OS
architecture
available_parallelism
debug/release
provider adapter
logical model identity
model revision nếu ResolvedModel expose được
num_threads
workload version/hash
delivery mode
```

Có thể thử lấy git SHA:

```rust
std::process::Command::new("git")
    .args(["rev-parse", "HEAD"])
```

Nếu không có `.git`, dùng:

```text
null
```

hoặc build-time env nếu CI inject.

Không fail benchmark chỉ vì không lấy được git metadata.

---

# 44. Secret handling và sanitized errors

Không serialize:

```text
OpenAI API key
tokens
authorization headers
device credentials
```

LLM report chỉ nên ghi:

```text
adapter
model
base URL origin/host nếu cần
timeout
```

Nếu base URL có query chứa secret thì sanitize trước khi ghi.

Raw `anyhow`, `reqwest`, provider error hay `Debug` representation không được serialize vào JSON.
Failure report dùng taxonomy hữu hạn do benchmark sở hữu:

```text
configuration
model_preparation
provider_initialization
provider_failure
timeout
capacity
invalid_output
workload
internal
```

`message` được tạo từ mapping/redaction của benchmark, không phải `error.to_string()`. Terminal
mặc định dùng cùng sanitized message. `--debug-errors` chỉ được in chi tiết đã redact ra terminal/
stderr và không được đưa vào JSON artifact.

---

# 45. Terminal report

Ví dụ TTS:

```text
TTS provider benchmark
adapter: zerotts_onnx
mode: delivery
workload: tts-v1
iterations: 20 x 3 items

Metric                 p50       p95       p99
------------------------------------------------
First PCM            112 ms    128 ms    132 ms
First Opus           129 ms    145 ms    151 ms
Synthesis+delivery   701 ms    755 ms    770 ms
Delivery total       705 ms    760 ms    776 ms
RTF synth+delivery    0.21      0.23      0.24
RTF delivery          0.21      0.24      0.24

Success: 60 / 60
```

Không sort provider và tuyên bố “best” nếu môi trường/workload không tương đương.

---

# 46. Runtime benchmark: TTS

Sau provider benchmark mới thêm runtime benchmark.

Production:

```text
TtsWorkerRuntime
  ├── tts-native-0
  ├── tts-native-1
  ├── ...
  └── tts-native-N
```

Mỗi worker mở native TTS state một lần.

Benchmark concurrency:

```text
1
2
4
max_workers
max_workers + 1
```

Metrics:

```text
accepted
capacity_rejected
request latency
first PCM
throughput
active leases peak
failure
timeout
cleanup timeout
```

---

# 47. Warm toàn bộ TTS pool trước steady runtime benchmark

`TtsWorkerRuntime::new()` spawn worker thread nhưng không có readiness barrier public.

Mỗi worker thread gọi `provider.open_worker()` trước command loop.

Nếu đo request đầu tiên ngay sau `new()`, latency có thể chứa native worker initialization.

Để steady runtime benchmark không lẫn chi phí đó:

1. Tạo `max_workers` requests đồng thời.
2. Buộc chúng chiếm các worker khác nhau.
3. Drain terminal event.
4. Discard toàn bộ warmup samples.
5. Sau đó mới vào measured phase.

Nếu sau này cần metric chính xác `worker_pool_ready_ms`, hãy thêm readiness barrier/event vào runtime thay vì suy đoán bằng sleep.

**Không dùng `sleep(500ms)` để giả định worker đã ready.**

---

# 48. Runtime benchmark: ASR

`AsrWorkerRuntime::open()` hiện:

```text
allocate lease
spawn OS thread
provider.open()
```

cho từng stream.

Do đó runtime benchmark ASR nên đo cả:

```text
open request → Opened event
audio push
Finish → Final
```

Đây là production behavior hiện tại.

Nếu benchmark cho thấy thread spawn/open cost đáng kể, đó là dữ liệu để cân nhắc fixed ASR pool về sau; benchmark không được tự đổi architecture.

---

# 49. Runtime benchmark: VAD

Production VAD worker nhận:

```text
960-sample uplink
```

rồi `VadRechunker` tạo:

```text
512-sample provider frames
```

Runtime benchmark phải feed 960, không feed trực tiếp 512.

Metrics:

```text
open latency
probability event latency
processed audio duration
worker capacity
reset latency
cleanup latency
```

---

# 50. Runtime benchmark: LLM

`LlmRuntime` dùng:

```rust
Semaphore
```

giới hạn concurrency.

Test:

```text
capacity = C

C requests       → tất cả phải được accept nếu không có lỗi provider
C + 1 immediate  → request vượt capacity phải trả Capacity
```

Ngoài correctness, đo:

```text
runtime start overhead
TTFT
total
capacity rejection count
```

Nhưng remote provider variability vẫn phải được ghi chú.

---

# 51. E2E load test

Không đặt E2E implementation trong `Provider Benchmark`.

Khuyến nghị thêm binary ở:

```text
crates/voice-reference-client/src/bin/voice-load.rs
```

Virtual client:

```text
connect WS
  ↓
hello
  ↓
listen:start
  ↓
send canonical uplink Opus 60 ms
  ↓
listen:stop hoặc Auto endpoint
  ↓
observe ASR/LLM/TTS control
  ↓
first binary Opus
  ↓
tts:stop
```

Metrics:

```text
utterance_terminal → first assistant control
utterance_terminal → tts:start
utterance_terminal → first binary audio
utterance_terminal → tts:stop
turn total
connection failure
protocol failure
```

Đây là lớp để load test:

```text
1
2
4
8
...
```

Voice Sessions.

---

# 52. Không đo playback trong server benchmark

Boundary server kết thúc khi packet đã sẵn sàng/gửi xong theo layer.

Không đo:

```text
speaker DAC
client jitter buffer
ESP32 playback scheduler
physical audio heard
```

Các metric đó thuộc HIL/client playback benchmark.

---

# 53. Error model

Tạo error enum riêng:

```rust
#[derive(Debug, thiserror::Error)]
pub enum BenchmarkError {
    #[error("configuration failed: {0}")]
    Config(String),

    #[error("workload failed: {0}")]
    Workload(String),

    #[error("provider initialization failed: {0}")]
    ProviderInit(String),

    #[error("qualification failed: {0}")]
    Qualification(String),

    #[error("TTS worker reset failed")]
    TtsReset,

    #[error("report failed: {0}")]
    Report(String),
}
```

Per-sample provider failure không nhất thiết return ngay; ghi vào raw samples.

Fatal errors:

```text
cannot parse config
cannot parse workload
model missing
provider cannot build
qualification fails
TTS worker reset fails
cannot create report file
```

Measured request failure:

```text
record failure
continue next sample
```

Sau run:

- vẫn write report;
- nếu `--fail-on-error` và có failure → exit non-zero.

---

# 54. Timeout

Không dùng cùng một timeout cho mọi target.

Provider local:

```text
VAD: normally no async timeout needed, but fixture/run watchdog may exist
ASR: final timeout configurable
TTS: use TTS config timeout or explicit benchmark timeout
```

LLM remote:

```text
use provider timeout
```

Runtime benchmark phải dùng production runtime timeout semantics khi mục tiêu là runtime fidelity.

CLI có thể thêm:

```text
--timeout-ms
```

chỉ khi override được ghi rõ trong report.

---

# 55. Không để benchmark logging phá timing

Default:

```text
RUST_LOG=warn
```

Không log mỗi PCM chunk trong measured loop.

Nếu production provider đang `info!` theo utterance thì chấp nhận, nhưng benchmark không nên bổ sung per-frame logs.

Raw data lưu vào memory rồi write JSON sau measured phase, không flush file từng sample nếu việc đó có thể ảnh hưởng local inference.

---

# 56. CPU scheduling và repeatability

Trước khi kết luận regression:

1. Dùng `--release`.
2. Không chạy heavy build song song.
3. Giữ `num_threads` giống nhau.
4. Dùng cùng model artifact/revision.
5. Dùng cùng workload SHA.
6. Dùng cùng máy hoặc hardware class.
7. Chạy đủ iterations.
8. So p50/p95 thay vì một lần đo.
9. Không so run đang download model với warm-cache startup.
10. Ghi rõ backend CPU hiện tại.

Không cố pin CPU affinity trong phiên bản đầu; có thể bổ sung sau nếu cần.

---

# 57. CI strategy

## Luôn chạy

Unit tests cho:

```text
stats
workload parser
report serialization
target-scoped validation
DownlinkDeliveryEncoder
tail handling
qualification validators
```

Các test này không cần model thật.

## Opt-in / hardware runner

Real-model benchmark:

```text
VAD
ASR
ZeroTTS
```

chạy trên runner đã chuẩn bị artifact.

## Remote integration

LLM performance:

```text
manual/scheduled
```

không nằm trong deterministic PR gate mặc định.

---

# 58. Regression gate

Không hard-code threshold ngay trong implementation đầu tiên.

Trước hết thu baseline.

Sau đó mới thêm compare command:

```bash
performance-tester compare \
  --baseline baseline.json \
  --candidate candidate.json
```

Compare phải reject nếu khác:

```text
target
mode
workload_version
workload_sha256
provider adapter
logical model identity/revision
num_threads
architecture
```

trừ khi user dùng explicit override.

Ví dụ policy về sau:

```text
TTS first_pcm p95 regression > X%
TTS delivery RTF p95 regression > Y%
ASR compute RTF p95 regression > Z%
VAD frame p95 regression > W%
```

Threshold phải được quyết định sau khi có noise floor thực tế của hardware runner.

---

# 59. Implementation phases

## Phase PT-0 — Contracts và skeleton

Thêm:

```text
src/performance_tester/
src/bin/performance-tester.rs
benchmarks/performance_tester/
```

Hoàn thành:

- CLI parse;
- workload schema;
- report schema;
- stats;
- environment metadata;
- JSON output.

**DoD**

```text
cargo test
cargo run ... --help
```

đều thành công.

---

## Phase PT-1 — Target-scoped config/provider loader

Refactor:

```text
AppConfig::parse_and_resolve
AppConfig::load
AppConfig::load_for_benchmark
validate_for_benchmark
```

Thêm:

```text
load_vad_for_benchmark
load_asr_for_benchmark
load_llm_for_benchmark
load_tts_for_benchmark
```

**DoD**

TTS benchmark config không bị fail vì ASR/LLM/VAD unrelated config.

Production startup vẫn full validate/load như trước.

---

## Phase PT-2 — TTS provider benchmark

Implement:

```text
provider tts --mode provider
```

Metrics:

```text
provider build/warmup
worker open
first PCM
total
audio duration
RTF
x realtime
chunk count
```

**DoD**

Run được ZeroTTS real model với nhiều workload items và xuất JSON.

---

## Phase PT-3 — Production downlink extraction

Tạo:

```text
audio::DownlinkDeliveryEncoder
```

Refactor `SpeechOutput` dùng class này.

Giữ:

```text
fade-in
stateful resampler
1440 framing
Opus options
fade-out tail
zero-pad
```

**DoD**

Tất cả SpeechOutput tests cũ pass và có test mới cho tail/equivalence.

---

## Phase PT-4 — TTS delivery benchmark

Implement:

```text
provider tts --mode delivery
```

Metrics:

```text
first PCM
first Opus
synthesis with delivery
delivery total
Opus packet count
Opus bytes
RTF synthesis-with-delivery/delivery
```

**DoD**

Delivery dùng đúng PCM callbacks của cùng một synthesis và đúng production converter.

---

## Phase PT-5 — ASR provider benchmark

Implement:

```text
provider asr --feed burst
provider asr --feed realtime
```

Metrics:

```text
session open
first partial
finish
active compute
wall time
RTF
```

**DoD**

Zipformer fixture chạy được và final text qualification pass.

---

## Phase PT-6 — VAD provider benchmark

Implement:

```text
provider vad
```

Metrics:

```text
frame p50/p95/p99
FPS
RTF
```

**DoD**

Timeline 512 samples được validate.

---

## Phase PT-7 — LLM provider benchmark

Implement:

```text
provider llm
```

Metrics:

```text
TTFT
total
text chunks
chars
tool-call count
error/timeout
```

**DoD**

JSON không chứa API key và TTFT chỉ tính first non-empty text delta.

---

## Phase PT-8 — Runtime concurrency benchmark

Implement:

```text
runtime vad
runtime asr
runtime llm
runtime tts
```

**DoD**

Có capacity test và concurrency matrix.

---

## Phase PT-9 — E2E load generator

Implement trong Reference Client:

```text
voice-load
```

**DoD**

N virtual clients chạy real WebSocket voice flow và report first downlink Opus/turn latency.

---

# 60. TTS implementation sketch

Pseudo-code provider mode:

```rust
fn run_tts_provider_sample(
    worker: &mut dyn TtsWorker,
    text: &str,
) -> Result<TtsSample, BenchmarkError> {
    let cancelled = AtomicBool::new(false);

    let started = Instant::now();
    let mut first_pcm = None;
    let mut pcm_chunks = 0usize;
    let mut pcm_samples = 0usize;

    worker.synthesize(text, &cancelled, &mut |pcm| {
        validate_tts_pcm(&pcm)?;

        if first_pcm.is_none() {
            first_pcm = Some(started.elapsed());
        }

        pcm_chunks += 1;
        pcm_samples += pcm.samples().len();

        Ok(())
    })?;

    let provider_total = started.elapsed();

    let audio_duration =
        Duration::from_secs_f64(pcm_samples as f64 / 48_000.0);

    Ok(TtsSample {
        first_pcm,
        provider_total,
        pcm_chunks,
        pcm_samples,
        audio_duration,
        rtf_provider:
            provider_total.as_secs_f64()
                / audio_duration.as_secs_f64(),
        // ...
    })
}
```

---

# 61. TTS delivery implementation sketch

```rust
fn run_tts_delivery_sample(
    worker: &mut dyn TtsWorker,
    text: &str,
) -> Result<TtsSample, BenchmarkError> {
    let cancelled = AtomicBool::new(false);
    let mut delivery = DownlinkDeliveryEncoder::new(4_000)?;

    let started = Instant::now();

    let mut first_pcm = None;
    let mut first_opus = None;

    let mut pcm_chunks = 0usize;
    let mut pcm_samples = 0usize;

    let mut opus_packets = 0usize;
    let mut opus_bytes = 0usize;

    worker.synthesize(text, &cancelled, &mut |pcm| {
        validate_tts_pcm(&pcm)?;

        if first_pcm.is_none() {
            first_pcm = Some(started.elapsed());
        }

        pcm_chunks += 1;
        pcm_samples += pcm.samples().len();

        for packet in delivery.push_provider_pcm(pcm)? {
            if first_opus.is_none() {
                first_opus = Some(started.elapsed());
            }

            opus_packets += 1;
            opus_bytes += packet.as_bytes().len();
        }

        Ok(())
    })?;

    let synthesis_with_delivery = started.elapsed();

    for packet in delivery.finish()? {
        if first_opus.is_none() {
            first_opus = Some(started.elapsed());
        }

        opus_packets += 1;
        opus_bytes += packet.as_bytes().len();
    }

    let delivery_total = started.elapsed();

    // derive duration / RTF ...

    Ok(/* sample */)
}
```

Quan trọng:

```text
delivery.finish()
```

nằm **sau** worker synthesize return và là một phần của `delivery_total`.

Caller phải gọi `worker.reset()?` sau **mọi** warmup, qualification và measured sample trước item
kế tiếp, kể cả khi sample đo được bị ghi nhận failure. Reset failure là fatal cho run `tts-v1`.
Ví dụ vòng điều phối measured phải có reset tách khỏi việc persist sample:

```rust
for item in &workload.items {
    let sample = measure_tts_item(worker.as_mut(), item, mode);
    worker.reset().map_err(|_| BenchmarkError::TtsReset)?;
    samples.push(record_sample(sample));
}
```

---

# 62. ASR implementation sketch

```rust
fn run_asr_burst(
    provider: &dyn AsrProvider,
    samples: &[f32],
) -> Result<AsrSample, BenchmarkError> {
    let open_started = Instant::now();
    let mut session = provider.open()?;
    let session_open = open_started.elapsed();

    let wall_started = Instant::now();

    let mut compute_total = Duration::ZERO;
    let mut first_partial_wall = None;
    let mut first_partial_compute = None;

    for chunk in samples.chunks(960) {
        if chunk.len() != 960 {
            // fixture policy: reject or documented final partial chunk
            break;
        }

        let started = Instant::now();
        let events = session.push_pcm(
            &PcmF32Mono::new(chunk.to_vec(), 16_000)
        )?;
        compute_total += started.elapsed();

        if first_partial_wall.is_none()
            && events.iter().any(is_non_empty_partial)
        {
            first_partial_wall = Some(wall_started.elapsed());
            first_partial_compute = Some(compute_total);
        }
    }

    let finish_started = Instant::now();
    let result = session.finish()?;
    let finish = finish_started.elapsed();
    compute_total += finish;

    let wall_total = wall_started.elapsed();

    // validate result.text(), derive RTF

    Ok(/* sample */)
}
```

Realtime mode dùng deadline cadence, không đơn giản `sleep(60ms)` sau mỗi push vì compute time sẽ cộng dồn drift.

Nên:

```text
origin + frame_index * 60 ms
```

và sleep đến deadline kế tiếp.

---

# 63. Realtime feed cadence

Pseudo:

```rust
let origin = Instant::now();

for (index, chunk) in chunks.enumerate() {
    let target = origin + Duration::from_millis(index as u64 * 60);

    if let Some(delay) = target.checked_duration_since(Instant::now()) {
        std::thread::sleep(delay);
    }

    push(chunk);
}
```

Nếu provider compute chậm hơn cadence, không sleep âm; benchmark sẽ thể hiện lag.

Có thể report:

```text
realtime_lag_max_ms
```

về sau.

---

# 64. LLM implementation sketch

```rust
let started = Instant::now();

let mut stream = provider.stream(request).await?;

let mut first_text = None;
let mut text_chunks = 0usize;
let mut output_chars = 0usize;
let mut tool_calls = 0usize;

while let Some(event) = stream.next().await {
    match event? {
        LlmEvent::TextDelta(text) => {
            if !text.trim().is_empty() && first_text.is_none() {
                first_text = Some(started.elapsed());
            }

            text_chunks += 1;
            output_chars += text.chars().count();
        }

        LlmEvent::ToolCall(_) => {
            tool_calls += 1;
        }

        LlmEvent::Finished => {
            break;
        }
    }
}

let total = started.elapsed();
```

Nếu workload text-only mà:

```text
first_text == None
```

qualification/sample fail.

---

# 65. VAD implementation sketch

```rust
let mut session = provider.open()?;

for (index, chunk) in samples.chunks_exact(512).enumerate() {
    let input = VadInput {
        pcm: chunk.to_vec(),
        start_sample: (index * 512) as u64,
    };

    let started = Instant::now();
    let probability = session.push(input)?;
    let elapsed = started.elapsed();

    validate_probability(probability)?;
    samples_us.push(elapsed.as_secs_f64() * 1_000_000.0);
}
```

---

# 66. Tests bắt buộc trước khi tin số benchmark

## Stats

```text
p50 đúng
p95 đúng
p99 đúng
empty input
single sample
mean
```

## Workload

```text
schema version unsupported → fail
duplicate item id → fail
empty workload → fail
missing fixture → fail
hash stable
```

## TTS delivery

```text
invalid sample rate → fail
NaN PCM → fail
fade-in only once
stateful resampler across arbitrary chunks
full frame emits packet
tail emits final packet
tail zero-padding
packet size cap
tts-v1 reset sau warmup, qualification và measured sample
reset failure dừng run
```

## Target loader

```text
TTS target không validate ASR
ASR target không build TTS
LLM target không Model Prepare local TTS
production AppConfig::load vẫn full validation
```

## Report

```text
secret và raw provider error không serialized
failure dùng error_kind/error_code/message sanitized
failure sample retained
warmup không nằm trong steady summary
JSON round-trip
```

---

# 67. Những điều không nên làm

## Không benchmark bằng `cargo test --release` rồi lấy test duration

Test harness duration không phải provider metric.

## Không benchmark debug build

Local ONNX/native inference phải dùng:

```text
--release
```

## Không mở model lại mỗi iteration

Cold-start và steady-state là hai metric khác nhau.

## Không sleep để “chờ worker ready”

Dùng real readiness/warmup semantics.

## Không dùng một WebSocket test để kết luận ZeroTTS chậm

WebSocket test chứa quá nhiều lớp.

## Không bỏ final TTS tail

Sẽ làm delivery benchmark lệch production.

## Không ghi API key vào JSON report

Report thường được lưu CI artifact.

## Không chỉ report average

Phải có tail latency.

## Không silently transform workload

Profile mismatch phải fail hoặc transform phải là một target được khai báo rõ.

---

# 68. Definition of Done tổng thể

`performance_tester` phase provider được coi là hoàn thành khi:

1. Có CLI native Rust.
2. VAD/ASR/LLM/TTS có workload versioned.
3. Có target-scoped config validation.
4. Chỉ Model Prepare provider đang benchmark.
5. Initialization và steady metrics tách biệt.
6. Warmup/qualification không đi vào percentile.
7. TTS provider dùng persistent `TtsWorker`.
8. TTS delivery reuse production downlink converter.
9. Delivery giữ fade-in, stateful resample, fade-out 480 downlink samples/20 ms, 1,440-frame, zero-pad và Opus options production.
10. ASR có `burst` và `realtime` tách riêng.
11. ASR RTF dùng active compute, không dùng realtime sleep.
12. LLM TTFT là first non-empty `TextDelta`.
13. Report có raw samples + p50/p95/p99.
14. Không tự drop outlier.
15. Report không chứa secret hoặc raw provider error; failure dùng taxonomy/mapping sanitized.
16. JSON ghi workload hash và environment fingerprint.
17. Unit tests bảo vệ benchmark contracts.
18. `cargo test` pass.
19. Real-model smoke chạy được ít nhất VAD/ASR/ZeroTTS trên hardware chuẩn.
20. Production server startup/behavior không thay đổi ngoài refactor shared deterministic audio conversion.

---

# 69. Thứ tự commit khuyến nghị

Để review dễ, không nên đưa tất cả vào một commit.

### Commit 1

```text
feat(perf): add benchmark workload, stats and report foundation
```

### Commit 2

```text
refactor(config): add target-scoped benchmark validation
```

### Commit 3

```text
feat(perf): add TTS provider benchmark
```

### Commit 4

```text
refactor(audio): extract production downlink delivery encoder
```

### Commit 5

```text
feat(perf): add TTS delivery benchmark
```

### Commit 6

```text
feat(perf): add ASR provider benchmark
```

### Commit 7

```text
feat(perf): add VAD provider benchmark
```

### Commit 8

```text
feat(perf): add LLM provider benchmark
```

### Commit 9

```text
feat(perf): add worker runtime concurrency benchmarks
```

### Commit 10

```text
feat(load): add reference-client voice load benchmark
```

---

# 70. Ưu tiên thực tế cho project hiện tại

Với trạng thái server hiện nay, ưu tiên nên là:

```text
PT-0 foundation
    ↓
PT-1 scoped loader
    ↓
PT-2 TTS provider
    ↓
PT-3 DownlinkDeliveryEncoder extraction
    ↓
PT-4 TTS delivery
    ↓
PT-5 ASR
    ↓
PT-6 VAD
    ↓
PT-7 LLM
```

Lý do TTS đi trước:

- ZeroTTS local là workload nặng nhất hiện tại.
- Project vừa có các thay đổi liên quan TTS streaming/pacing/tail.
- Cần phân biệt model synthesis chậm với resample/Opus/delivery chậm.
- `SpeechOutput` hiện chứa deterministic conversion cần tách để tránh benchmark drift.
- Sau khi có TTS provider + delivery, việc tìm regression audio sẽ chính xác hơn nhiều so với đo WebSocket end-to-end duy nhất.

---

# 71. Kết luận kiến trúc

Không xây `performance_tester` như một bản Rust của các script Python Xiaozhi.

Nên xây theo hierarchy:

```text
Provider Benchmark
    │
    ├── VAD
    ├── ASR
    ├── LLM
    └── TTS
         ├── provider PCM
         └── delivery Opus

Worker Runtime Benchmark
    │
    ├── VAD workers
    ├── ASR workers
    ├── LLM semaphore
    └── TTS fixed native pool

Protocol E2E Load Test
    │
    └── Reference Client → WebSocket → SessionActor → providers → Opus
```

Nguyên tắc quan trọng nhất:

> **Benchmark phải đo đúng production boundary nhưng không kéo thêm layer không thuộc metric đang đo.**

Với TTS:

```text
provider mode
    text → provider-facing PCM

delivery mode
    text → cùng PCM stream → production deterministic conversion → final Opus packet ready
```

Với ASR:

```text
provider compute ≠ realtime wall latency
```

Với LLM:

```text
TTFT ≠ total latency
```

Với runtime:

```text
provider speed ≠ server concurrency capacity
```

Và với E2E:

```text
server packet ready ≠ client physical playback complete
```

Giữ các boundary này tách biệt sẽ giúp benchmark có thể dùng lâu dài để tối ưu provider, phát hiện regression giữa commit và quyết định capacity/runtime dựa trên dữ liệu thay vì cảm nhận.
