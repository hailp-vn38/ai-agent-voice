# ZeroTTS Runtime Optimization Guide

## Mục tiêu

Tài liệu này hướng dẫn refactor ZeroTTS runtime trong `ai-agent-voice` để giảm:

- thời gian materialize/load runtime;
- RAM do duplicate ONNX session;
- I/O do verify artifact lặp lại;
- chi phí warmup khi provider được load;

đồng thời vẫn giữ các invariant quan trọng:

- một physical ZeroTTS runtime có thể phục vụ nhiều logical voice/provider;
- voice không làm thay đổi physical `ResourceKey`;
- không share mutable turn/codec state giữa các Voice Session;
- không reload ZeroTTS khi chỉ đổi Template/voice;
- ONNX inference vẫn chạy ngoài Tokio executor;
- runtime chỉ được công bố `Ready` sau khi retained native resources thực sự usable.

Repo mục tiêu:

- `https://github.com/hailp-vn38/ai-agent-voice`
- branch triển khai: `dev-test`

---

# 1. Vấn đề hiện tại

## 1.1 Một ZeroTTS worker đang sở hữu bốn ONNX sessions

Ở `crates/voice-agent-server/src/providers/tts/zerotts/runtime/synthesis.rs`, `ZeroTtsOperation::new()` load ba graph:

- `text_encoder`
- `prefix_step`
- `local_frame_decode`

Ở stream mode, codec load thêm:

- `codec_decode_step`

Ở file mode, codec dùng:

- `codec_decode_full`

Do đó một native ZeroTTS worker hiện tương đương:

```text
1 worker
  = text_encoder Session
  + prefix_step Session
  + local_frame_decode Session
  + 1 codec Session
  = 4 ONNX sessions
```

Nếu:

```toml
[workers.tts]
max_workers = 2
```

thì một physical ZeroTTS runtime có thể resident tới tám ONNX sessions.

Đây là nguyên nhân chính làm tăng startup latency và RAM.

---

## 1.2 Generic `workers.tts.max_workers` đang vô tình quyết định số bản copy của ZeroTTS engine

Hiện `TtsWorkerRuntime` tạo native worker theo:

```rust
for index in 0..config.max_workers {
    // spawn worker
}
```

Mỗi worker gọi:

```rust
provider.open_worker()
```

và `ConfiguredZeroTts::open_worker()` tạo một bộ native inference state mới.

Điều này đang trộn hai khái niệm khác nhau:

1. application/global TTS concurrency;
2. số physical ZeroTTS engine replicas.

Hai giá trị này không nên mặc định bằng nhau.

---

## 1.3 Native worker initialization hiện tuần tự

`TtsWorkerRuntime::try_new_with_admission_and_binding()` hiện có flow tương đương:

```text
spawn worker 0
wait worker 0 Ready
spawn worker 1
wait worker 1 Ready
...
```

Mỗi worker lại thực hiện:

```text
open_worker()
  -> load ONNX sessions
warmup()
  -> full ZeroTTS synthesis
```

Vì vậy load time tăng gần tuyến tính theo `max_workers`.

Không sửa bằng cách parallel-init tất cả worker ngay. Parallel session load có thể làm tăng peak RAM rất lớn và chỉ che nguyên nhân kiến trúc gốc.

---

## 1.4 Warmup ZeroTTS hiện quá nặng

`ZeroTtsNativeWorker::warmup()` hiện synthesize câu:

```text
ZeroTTS startup readiness.
```

và chạy gần như full TTS pipeline tới PCM.

Mục đích của startup warmup chỉ cần:

- graph/session thực sự load được;
- lazy allocator/thread-pool của ORT được kích hoạt;
- từng graph trên hot path chạy được;
- codec sinh được finite/non-empty PCM;
- retained worker được reset sạch trước traffic.

Không cần synthesize một utterance hoàn chỉnh ở mỗi runtime load.

---

## 1.5 Artifact preparation/verification có thể bị thực hiện lặp

Trong `crates/voice-agent-server/src/services/provider_runtime/factory.rs`:

- `prepare_artifacts()` gọi `prepare_immutable()`;
- `build()` lại gọi `prepare_immutable()` để lấy `ResolvedModel`.

Trong `models.rs`, immutable artifact cuối cùng vẫn được `verify_path()` SHA-256.

Với ZeroTTS có nhiều artifact lớn, việc đọc/hash lại model trong cùng materialization là I/O không cần thiết.

---

# 2. Kiến trúc mục tiêu

## 2.1 Một physical ZeroTTS runtime phục vụ nhiều logical voice

Giữ mô hình:

```text
                     Physical ZeroTTS Runtime
                +--------------------------------+
                | Shared immutable/static data   |
                |                                |
                | config                         |
                | tokenizer                      |
                | voices registry                |
                | graph metadata                 |
                | graph/artifact paths           |
                +---------------+----------------+
                                |
                +---------------v----------------+
                | Engine Replica #0              |
                |                                |
                | text_encoder Session           |
                | prefix_step Session            |
                | local_frame_decode Session     |
                | codec session                  |
                +---------------+----------------+
                                |
                     request / turn state
                                |
                +---------------v----------------+
                | ZeroTtsTurnState               |
                | voice Arc<embedding>           |
                | packed_kv                      |
                | cross_kv                       |
                | text_valid/full_valid          |
                | seen_mask                      |
                | codec KV/cache                 |
                +--------------------------------+
```

Default cho homelab:

```text
ZeroTTS physical replicas = 1
```

Nhiều provider logical:

```text
tts_maichi
tts_hamy
tts_baotrang
tts_giahuy
```

phải reuse cùng physical runtime khi model/execution settings giống nhau.

---

## 2.2 Không đưa voice vào physical identity

Hiện `ZeroTtsPhysicalSpec` chỉ chứa `delivery_mode` là đúng hướng.

Phải tiếp tục đảm bảo `ResourceKey` KHÔNG chứa:

- `voice`;
- `language` nếu chỉ là logical selection;
- provider database id;
- Template id;
- Agent id;
- preload flag.

Physical identity chỉ được phụ thuộc vào state thực sự làm thay đổi native resource, ví dụ:

```text
adapter
model/artifact fingerprint
ONNX runtime fingerprint
execution threads
codec/delivery mode
physical replica topology
```

Nếu hai Template khác nhau chỉ khác voice:

```text
Template A -> maichi
Template B -> hamy
```

thì `server.switch_template` chỉ đổi logical `TtsBinding`; không được unload/reload/warmup ZeroTTS.

---

# 3. Thay đổi P0 - Giảm ZeroTTS physical replicas về 1

## Mục tiêu

Không để `workers.tts.max_workers` tự động tạo nhiều bản copy của ZeroTTS engine.

## Yêu cầu

Tách:

```text
Global/application TTS concurrency
```

khỏi:

```text
ZeroTTS physical engine replicas
```

### Phương án khuyến nghị

Bổ sung capacity vật lý vào local runtime plan thay vì để generic `TtsWorkerRuntime` tự suy luận.

Có thể mở rộng:

```rust
pub struct LocalRuntimePlan {
    ...
    physical_capacity: usize,
}
```

hoặc một abstraction tương đương.

Với ZeroTTS:

```rust
physical_capacity = 1
```

mặc định.

Nếu cần config operator-only về sau:

```toml
[runtime.zerotts]
replicas = 1
```

Không expose field này trong provider descriptor/Admin provider config.

## Không làm

Không thêm `threads`, `replicas`, `ws_url` hoặc low-level execution setting vào provider form của user.

---

# 4. Thay đổi P1 - Refactor ZeroTTS engine và turn state

## 4.1 Đổi tên và tách trách nhiệm

`ZeroTtsOperation` hiện chứa ONNX `Session`, do đó thực tế gần với resident engine hơn là một operation ngắn hạn.

Refactor theo hướng:

```rust
struct ZeroTtsReplica {
    text_encoder: Session,
    prefix_step: Session,
    local_frame_decode: Session,
    codec: ZeroTtsCodecReplica,
}
```

Shared immutable state:

```rust
struct ZeroTtsStatic {
    tokenizer: Tokenizer,
    config: Config,
    voices: Arc<ZeroTtsVoiceRegistry>,
    silence_frame: Vec<i32>,
    graphs: GraphPaths,
}
```

Request/turn-local state:

```rust
struct ZeroTtsTurnState {
    // exact concrete tensor types may remain implementation detail
    packed_kv: ...,
    full_valid: ...,
    cross_kv: ...,
    text_valid: ...,
    seen_mask: ...,
    codec_state: ...,
}
```

Không bắt buộc phải đưa tất cả state vào một struct duy nhất ngay ở commit đầu tiên, nhưng ownership cuối cùng phải rõ:

```text
Session/model weights -> physical replica
KV/cache/random/history -> current turn/stream only
Voice embedding -> shared immutable Arc
```

---

## 4.2 Session lifecycle

Session phải được tạo đúng một lần khi physical replica được materialize:

```text
load runtime
  -> create 4 ONNX sessions
  -> warmup retained sessions
  -> Ready
```

Turn mới:

```text
lease replica
  -> create/reset turn state
  -> lookup voice embedding
  -> synthesize
  -> clear mutable state
  -> release replica
```

Turn mới KHÔNG được gọi lại `commit_from_file()`.

---

# 5. Thay đổi P1 - Fast retained-worker warmup

## Mục tiêu

Thay full-sentence synthesis warmup bằng bounded one-step warmup vẫn exercise toàn bộ hot path.

## Flow yêu cầu

Warmup nên chạy tối thiểu:

```text
1. text_encoder       -> 1 call
2. prefix_step cold   -> 1 call
3. local_frame_decode -> 1 call
4. prefix_step frame  -> 1 call
5. codec decoder      -> 1 small decode call
6. validate finite/non-empty PCM
7. reset/drop all warmup mutable state
```

Không chạy loop tới EOA.

Upstream ZeroTTS sử dụng warmup theo tinh thần này: dummy text encoder, prefix cold start, one local frame decode và one prefix frame step để kích hoạt các session hot-path.

## API gợi ý

Thay logic trong:

```rust
impl TtsWorker for ZeroTtsNativeWorker {
    fn warmup(&mut self) -> Result<(), TtsError>
}
```

bằng một helper riêng, ví dụ:

```rust
fn fast_warmup(&mut self) -> Result<(), TtsError>
```

Không dùng `synthesize()` full utterance làm warmup production.

## Full synthesis validation chuyển sang qualification/test

Full deterministic utterance gate vẫn phải tồn tại trong:

- `provider-bench`;
- installed-model qualification;
- provider test endpoint nếu cần;
- CI/manual real-model gate.

Không dùng full utterance làm startup warmup cho mỗi runtime materialization.

---

# 6. Thay đổi P1 - Prepared model cache

## Mục tiêu

Immutable artifacts chỉ được verify/resolve một lần trong application lifecycle cho cùng model fingerprint.

## Thiết kế

Tạo application-owned catalog/cache, ví dụ:

```rust
struct PreparedModelCatalog {
    models: HashMap<ModelResourceKey, Arc<ResolvedModel>>,
}
```

Key phải dựa trên immutable model identity/fingerprint, không dựa trên logical provider id.

Startup/model preparation:

```text
manifest
  -> acquire missing artifact
  -> transform if required
  -> SHA verify
  -> install content-addressed immutable tree
  -> ResolvedModel
  -> cache Arc<ResolvedModel>
```

Runtime materialization:

```text
resource requested
  -> lookup PreparedModelCatalog
  -> Arc<ResolvedModel>
  -> build native sessions
```

Không SHA lại toàn bộ model nếu cùng immutable `ResolvedModel` đã được trusted trong current process.

---

# 7. Thay đổi contract `prepare_artifacts()`

## Vấn đề

Contract hiện tại:

```rust
fn prepare_artifacts(
    &self,
    snapshot: &DesiredProvider,
) -> Result<(), RuntimeError>
```

làm mất kết quả preparation.

`build()` sau đó buộc phải resolve lại model.

## Mục tiêu

Preparation phải trả lại typed prepared context cho build.

Một hướng triển khai:

```rust
enum PreparedRuntime {
    Remote,
    Local {
        model: Arc<ResolvedModel>,
        resource_key: ResourceKey,
    },
}
```

và materializer API trở thành tương đương:

```rust
fn prepare(
    &self,
    snapshot: &DesiredProvider,
) -> Result<PreparedRuntime, RuntimeError>;

fn build(
    &self,
    snapshot: &DesiredProvider,
    prepared: PreparedRuntime,
    quota: ProviderRuntimeAdmission,
) -> Result<Arc<dyn RuntimeResource>, RuntimeError>;
```

Không bắt buộc dùng chính tên/type trên, nhưng phải đạt invariant:

> model/artifact preparation của một materialization không bị thực hiện lại trong `build()`.

---

# 8. Reuse ONNX runtime fingerprint

`FactoryMaterializer::new()` đã capture `onnx_fingerprint`.

Không đọc/hash lại `runtime.onnx.library` trong hot materialization path nếu server process chưa thay đổi runtime binary.

`resource_key_with_fingerprint()` nên reuse fingerprint đã cache:

```rust
self.onnx_fingerprint
```

thay vì gọi lại `execution_file_fingerprint()` trong build path.

ONNX runtime library được xem immutable trong lifetime của process. Thay đổi runtime library yêu cầu restart server.

---

# 9. Voice registry

## Giữ eager voice registry

Không cần lazy-load từng voice ở giai đoạn này.

Các voice latent nhỏ so với ONNX model/session và đã có thể giữ dạng:

```rust
BTreeMap<String, Arc<Array3<f32>>>
```

Load/validate toàn bộ voice pack khi physical runtime được chuẩn bị là chấp nhận được.

Mục tiêu là tránh:

```text
voice A -> engine A
voice B -> engine B
```

và giữ:

```text
voice A --+
voice B --+--> one physical ZeroTTS runtime
voice C --+
```

---

# 10. File/module dự kiến cần sửa

Tối thiểu review và sửa các file sau.

## Provider/runtime planning

```text
crates/voice-agent-server/src/providers/local_runtime.rs
crates/voice-agent-server/src/services/provider_runtime/plan.rs
crates/voice-agent-server/src/services/provider_runtime/factory.rs
crates/voice-agent-server/src/services/provider_runtime/materialize.rs
crates/voice-agent-server/src/services/provider_runtime/registry.rs
```

## ZeroTTS native runtime

```text
crates/voice-agent-server/src/providers/tts/mod.rs
crates/voice-agent-server/src/providers/tts/zerotts/runtime/contract.rs
crates/voice-agent-server/src/providers/tts/zerotts/runtime/synthesis.rs
crates/voice-agent-server/src/providers/tts/zerotts/runtime/codec.rs
```

## Worker runtime

```text
crates/voice-agent-server/src/workers/tts/mod.rs
crates/voice-agent-server/src/workers/tts/pool.rs
```

## Model preparation/cache

```text
crates/voice-agent-server/src/models.rs
crates/voice-agent-server/src/models/startup.rs
```

Nếu cache được đặt ở app state, review thêm nơi application khởi tạo `FactoryMaterializer`/runtime manager.

---

# 11. Metrics bắt buộc bổ sung

Hiện đã có các phase tổng quát như:

```text
build
worker_init
warmup
```

Bổ sung timing chi tiết cho ZeroTTS materialization, ít nhất ở log/diagnostic không high-cardinality:

```text
artifact_prepare_ms
artifact_verify_ms
provider_contract_ms
worker_session_init_ms
worker_warmup_ms
runtime_total_ms
```

Không dùng provider DB id, revision hoặc resource hash làm metric label có cardinality không giới hạn.

Log ví dụ mong muốn:

```text
zerotts runtime materialized
artifact_prepare_ms=12
provider_contract_ms=25
worker_session_init_ms=2850
worker_warmup_ms=180
runtime_total_ms=3067
physical_replicas=1
```

Các timing phải cho phép xác nhận bottleneck sau refactor.

---

# 12. Invariant khi switch Template

Viết regression test cho case:

```text
Agent home
  Template A -> ZeroTTS maichi
  Template B -> ZeroTTS hamy
```

Cả hai có cùng:

```text
model
runtime library
threads
delivery_mode
physical capacity
```

Kỳ vọng:

```text
Template A acquire -> runtime build count +1
switch Template B  -> runtime build count không tăng
```

Chỉ logical view/binding thay đổi:

```rust
TtsBinding {
    voice: "hamy".into(),
    language: "vi-VN".into(),
}
```

Không:

- unload ZeroTTS;
- create new physical worker pool;
- create new ONNX sessions;
- rerun warmup.

---

# 13. Tests bắt buộc

## 13.1 Physical sharing test

Tạo hai `DesiredProvider` ZeroTTS khác voice nhưng cùng physical settings.

Assert:

```text
resource_key(A) == resource_key(B)
```

và manager chỉ build physical resource một lần.

---

## 13.2 Delivery mode test

Nếu:

```text
A.delivery_mode = stream
B.delivery_mode = file
```

thì:

```text
resource_key(A) != resource_key(B)
```

vì codec session topology khác nhau.

---

## 13.3 Warmup bounded test

Test fast warmup phải chứng minh:

- text encoder chạy;
- prefix cold chạy;
- local decode chạy;
- prefix frame step chạy;
- codec chạy;
- PCM finite;
- PCM non-empty;
- warmup không loop tới full utterance;
- reset thành công;
- request đầu sau warmup không reuse KV/cache của warmup.

---

## 13.4 Session reuse test

Instrument test provider/native wrapper để đếm session construction.

Với:

```text
1 physical replica
N sequential turns
```

expect:

```text
text_encoder construction       = 1
prefix_step construction        = 1
local_frame_decode construction = 1
codec construction              = 1
```

N turn không được làm count tăng.

---

## 13.5 Voice switch test

Synthesize hai request liên tiếp:

```text
turn 1 -> maichi
turn 2 -> hamy
```

Assert:

- cùng physical runtime;
- đúng binding tới từng voice;
- không rebuild runtime;
- turn 2 không nhận codec/AR state từ turn 1.

---

## 13.6 Artifact preparation test

Instrument preparation counter.

Một materialization phải có:

```text
prepare immutable model = 1 lần
```

không còn:

```text
prepare_artifacts -> prepare_immutable
build             -> prepare_immutable lần 2
```

---

## 13.7 Concurrency/admission test

Với ZeroTTS replicas = 1:

- chỉ một native synthesis được chạy tại một thời điểm trên physical engine;
- request thứ hai phải đi qua bounded admission/backpressure hiện có;
- không spawn engine replica ngầm;
- timeout không tạo replacement engine vô hạn;
- cleanup/quarantine semantics hiện tại vẫn giữ nguyên.

---

# 14. Benchmark trước và sau

Dùng cùng máy, model, ORT binary, thread count và filesystem cache condition.

Đo ít nhất:

```text
runtime cold materialization
runtime warm cache materialization
resident RSS sau Ready
time to first audio
full synthesis RTF
```

Case benchmark:

```text
A. current max_workers=2
B. optimized replicas=1
C. optimized replicas=2 (chỉ để so sánh)
```

Report tối thiểu:

| Metric | Before | After |
|---|---:|---:|
| artifact prepare | | |
| worker session init | | |
| warmup | | |
| total runtime load | | |
| RSS after load | | |
| TTFA | | |
| RTF | | |

Mục tiêu không phải hy sinh inference speed để giảm startup. Nếu RTF/TTFA xấu đáng kể phải điều tra trước khi merge.

---

# 15. Trình tự commit khuyến nghị

## Commit 1 - Tests/instrumentation trước

- thêm construction/build counters cho test;
- thêm detailed load timing;
- thêm regression test shared runtime giữa nhiều voice;
- ghi baseline runtime load/RSS.

Không đổi behavior lớn trong commit này.

## Commit 2 - Physical capacity = 1 cho ZeroTTS

- tách ZeroTTS physical capacity khỏi generic TTS capacity;
- giữ global admission semantics;
- đảm bảo ResourceKey capture physical capacity nếu capacity thực sự thay đổi backing pool.

## Commit 3 - Fast warmup

- thay full sentence startup warmup;
- giữ full model qualification ở benchmark/test gate;
- test reset/no-state-leak.

## Commit 4 - Prepared model handoff/cache

- preparation trả typed result;
- `build()` consume prepared result;
- bỏ duplicate `prepare_immutable()`;
- reuse ONNX fingerprint đã cache.

## Commit 5 - ZeroTTS engine/turn ownership cleanup

- rename/refactor `ZeroTtsOperation` nếu cần;
- tách rõ resident session và per-turn state;
- không thay output contract.

Không gộp toàn bộ thay đổi vào một commit lớn.

---

# 16. Điều không được làm

Không triển khai các shortcut sau:

```text
- runtime-per-voice
- runtime-per-template
- runtime-per-agent
- reload ZeroTTS khi switch_template chỉ đổi voice
- share codec KV/cache giữa Voice Session
- share mutable AR state giữa concurrent requests
- bỏ readiness/warmup hoàn toàn
- parallel-init nhiều engine để che load chậm
- đưa low-level ZeroTTS replica/thread settings ra Admin provider form
- bỏ artifact integrity validation trên installation/change boundary
- SHA/hash toàn bộ immutable model ở mọi runtime acquire
```

Không cố merge 3 TTS ONNX graph + codec graph thành một ONNX file trong scope này. Upstream ZeroTTS runtime contract vốn tách các graph này và pipeline dựa trên state giữa các call.

---

# 17. Acceptance criteria

Thay đổi chỉ được coi là hoàn tất khi đáp ứng tất cả điều kiện sau.

## Kiến trúc

- [ ] Multiple ZeroTTS voices cùng physical settings reuse một runtime.
- [ ] `voice` không nằm trong physical `ResourceKey`.
- [ ] Default ZeroTTS physical replica count là 1.
- [ ] Generic TTS concurrency không tự động nhân bản ZeroTTS engine.
- [ ] Session được giữ resident giữa các turn.
- [ ] Mutable KV/cache không cross Voice Session.

## Startup/runtime

- [ ] Một default ZeroTTS physical runtime chỉ tạo 4 ONNX sessions ở stream mode.
- [ ] Startup warmup exercise đủ hot-path graph nhưng không synthesize full utterance.
- [ ] Model preparation không chạy duplicate trong cùng materialization.
- [ ] ONNX runtime binary fingerprint không bị hash lại ở mỗi build.

## Switching

- [ ] Switch Template từ voice A sang voice B không rebuild ZeroTTS runtime.
- [ ] Runtime build counter không tăng khi chỉ đổi logical voice binding.

## Reliability

- [ ] Existing cancellation semantics giữ nguyên.
- [ ] Existing cleanup/quarantine semantics giữ nguyên.
- [ ] No native inference chạy trên Tokio executor.
- [ ] Failed native init không được báo `Ready`.
- [ ] Late/stale PCM không vượt generation boundary.

## Quality gate

- [ ] `cargo fmt --check`
- [ ] relevant ZeroTTS/unit tests
- [ ] provider runtime manager tests
- [ ] workspace tests phù hợp scope
- [ ] `git diff --check`
- [ ] real installed-model ZeroTTS qualification nếu môi trường có model/ORT
- [ ] benchmark before/after được lưu lại

---

# 18. Kết quả mong muốn cuối cùng

Đối với deployment homelab mặc định:

```text
1 ZeroTTS model
1 physical runtime
1 physical replica
4 resident ONNX sessions
1 fast warmup
N logical voices
N templates/agents có thể reuse runtime
```

Flow cuối cùng:

```text
Provider requested
      |
      v
compute physical ResourceKey
      |
      +---- HIT ----> reuse resident ZeroTTS runtime
      |
      `---- MISS
              |
              v
        resolve prepared model once
              |
              v
        create 4 ONNX sessions once
              |
              v
        bounded fast warmup
              |
              v
             Ready
              |
     +--------+---------+
     |        |         |
   maichi    hamy    baotrang
     |        |         |
     +--------+---------+
        logical binding only
```

Mục tiêu của refactor không phải giảm số graph của ZeroTTS. Mục tiêu là đảm bảo **mỗi physical replica chỉ load mỗi graph một lần**, và không để Agent/Template/provider logical configuration vô tình tạo thêm bản copy của model.
