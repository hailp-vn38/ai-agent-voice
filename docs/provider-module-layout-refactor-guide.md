# Hướng dẫn refactor module Provider theo Descriptor/Capability layout

## Mục đích

Tài liệu này là kế hoạch refactor cấu trúc mã, áp dụng module layout tại mục 2 của
[`provider-descriptor-capability-and-test-api-guide.md`](provider-descriptor-capability-and-test-api-guide.md).

Mục tiêu là để mỗi adapter sở hữu typed config, descriptor/capability, semantic
validation, factory/runtime builder và diagnostic implementation của chính nó. Refactor này
**không** thay đổi wire contract, Admin API, persisted provider row, lifecycle `RuntimeCatalog`,
hay ownership của Inference Worker Runtime và Voice Session.

## Invariants bắt buộc

- [ ] Giữ adapter ID, serde discriminator và giá trị `providers.adapter` hiện có:
  `zipformer_sherpa`, `gipformer_sherpa_offline`, `zerotts_onnx`, `chillaudio_ws`, `openai`.
- [ ] Giữ `providers.config_json` là typed, non-secret configuration; credential tiếp tục chỉ đi
  qua `secret_ref -> SecretResolver -> SecretValue`.
- [ ] Provider registry và factory vẫn compile-time; không thêm plugin discovery, runtime code
  loading, hay provider tự acquire model.
- [ ] Không đổi public API Admin hoặc key-based route cho descriptor, capability discovery và
  provider diagnostics.
- [ ] Không để adapter-specific JSON/implementation leak vào SessionActor hoặc Voice Session.
- [ ] Giữ public Rust path bằng `pub use` trong các module cha khi call site hiện hữu đang dùng
  path đó.

## Cấu trúc đích

```text
crates/voice-agent-server/src/providers/
├── registry.rs                 # ProviderAdapterRegistry: descriptor registry
├── factory_registry.rs         # ProviderRegistry: registry factory compile-time
├── descriptor.rs               # ProviderDescriptor và UI-neutral config schema
├── capabilities.rs             # capability value types và discovery mode
├── inspector.rs                # bootstrap/runtime capability inspection seams
├── diagnostics.rs              # provider-facing diagnostic seams
├── asr/
│   ├── mod.rs
│   ├── gipformer/{config.rs, descriptor.rs, provider.rs}
│   └── zipformer/{config.rs, descriptor.rs, provider.rs}
├── tts/
│   ├── mod.rs
│   ├── zerotts/{config.rs, descriptor.rs, provider.rs, ...}
│   └── chillaudio/{config.rs, descriptor.rs, provider.rs}
└── llm/
    ├── mod.rs
    └── openai/{config.rs, descriptor.rs, provider.rs}
```

`services/provider_diagnostic.rs` vẫn là application service: nó sở hữu lookup runtime,
limiter, timeout, terminal cleanup và quarantine. `providers/diagnostics.rs` chỉ chứa các seam
thuộc provider/adapter, không chuyển orchestration vào adapter.

## Hiện trạng và quyết định đặt tên

`providers/descriptor.rs` hiện đang chứa model descriptor, capability types, descriptor registry,
metadata của mọi adapter và bootstrap discovery. `providers/registry.rs` hiện lại là registry của
factory/runtime. Vì vậy chỉ đổi file mà không đổi vai trò sẽ gây hiểu lầm.

- [ ] Để `providers/registry.rs` dành cho `ProviderAdapterRegistry` đúng theo layout đích.
- [ ] Đổi file registry factory hiện tại thành `factory_registry.rs`.
- [ ] Giữ `ProviderRegistry` và `compiled_provider_registry()` là re-export tương thích từ
  `providers/mod.rs`; không bắt caller đổi tên trong cùng refactor.

## Checklist thực hiện

### 0. Chuẩn bị và baseline

- [ ] Ghi lại danh sách public re-export trong `providers/mod.rs`, `asr/mod.rs`, `tts/mod.rs`,
  `llm/mod.rs` trước khi di chuyển.
- [ ] Ghi lại tập adapter factory đăng ký trong `compiled_provider_registry()`.
- [ ] Ghi lại tập descriptor công bố trong `compiled_provider_adapter_registry()`.
- [ ] Chạy test module/provider hiện có trước refactor để phân biệt regression mới với baseline.

### 1. Tách các khái niệm dùng chung

- [ ] Chuyển `ProviderCapabilities`, `ModelOption`, `LanguageOption`, `VoiceOption`,
  `CapabilityDiscoveryMode`, `DiscoverySource` và `CapabilitySource` sang
  `providers/capabilities.rs`.
- [ ] Giữ `ProviderDescriptor`, `ProviderConfigSchema`, `ProviderConfigField`,
  `ConfigFieldType`, `ProviderType` và `AdapterSummary` trong `providers/descriptor.rs`.
- [ ] Chuyển `DiscoveredCapabilities`, `ProviderInspectError`, typed bootstrap selection và trait
  bootstrap/runtime inspector sang `providers/inspector.rs`.
- [ ] Đưa `ProviderAdapterRegistry::{list,get,supports}` vào `providers/registry.rs`; registry chỉ
  aggregate descriptor/inspector adapter công bố, không chứa metadata của adapter.
- [ ] Loại bỏ dispatch string tập trung như `match adapter { "zerotts_onnx" => ... }` khỏi
  registry; dispatch qua registration typed của adapter.

### 2. Refactor ASR adapter

- [ ] Tạo `providers/asr/zipformer/` và chuyển runtime implementation hiện ở
  `zipformer_sherpa.rs` vào `provider.rs`.
- [ ] Tạo `providers/asr/gipformer/` và chuyển runtime implementation hiện ở
  `gipformer_sherpa_offline.rs` vào `provider.rs`.
- [ ] Chuyển `ZipformerSherpaConfig` và `GipformerSherpaOfflineConfig`, default cùng semantic
  validation tương ứng vào `config.rs` của từng adapter.
- [ ] Đặt static model/language/audio capability và schema field của từng adapter tại
  `descriptor.rs` tương ứng.
- [ ] Để `asr/mod.rs` chỉ giữ ASR boundary trait, event/result public và re-export concrete type
  cần nội bộ.

### 3. Refactor TTS adapter

- [ ] Tạo `providers/tts/zerotts/config.rs`, `descriptor.rs`, `provider.rs`; giữ các module codec,
  contract, synthesis, text là implementation detail cùng thư mục `zerotts/`.
- [ ] Chuyển `ConfiguredZeroTts`, `ZeroTtsArtifacts` và ZeroTTS factory/build validation vào
  `zerotts/provider.rs`.
- [ ] Chuyển `ZeroTtsOnnxConfig` và `ZeroTtsDeliveryMode` vào `zerotts/config.rs`.
- [ ] Tạo `providers/tts/chillaudio/{config.rs,descriptor.rs,provider.rs}` từ
  `chillaudio_ws.rs` và config tương ứng.
- [ ] Giữ `TtsProvider`, `TtsStream`, `TtsWorker`, `TtsDiagnosticRequest` và `TtsError` là
  boundary chung ở `tts/mod.rs` hoặc module boundary nhỏ được re-export từ đó.
- [ ] Xác nhận ZeroTTS descriptor vẫn quảng bá PCM provider-boundary 48 kHz và Voice delivery
  24 kHz là hai capability khác nhau.

### 4. Refactor LLM adapter

- [ ] Tạo `providers/llm/openai/{config.rs,descriptor.rs,provider.rs}`.
- [ ] Chuyển `ConfiguredOpenAiLlm`, OpenAI mapping/stream adaptation và factory implementation
  vào `openai/provider.rs`.
- [ ] Chuyển `OpenAiConfig` và validation typed liên quan vào `openai/config.rs`.
- [ ] Giữ `LlmProvider`, `LlmRequest`, event/tool boundary ở `llm/mod.rs`; không đưa vendor type
  vào caller.

### 5. Wiring config và factory

- [ ] Giữ enum tổng hợp `AsrInstanceConfig`, `TtsInstanceConfig`, `LlmInstanceConfig` tại config
  boundary để deserialize deployment config.
- [ ] Để enum tổng hợp tham chiếu config type do adapter sở hữu, sau đó re-export type cũ từ
  `config` trong một commit tương thích nếu có caller nội bộ.
- [ ] Đăng ký factory từ từng adapter trong `factory_registry.rs`; file này chỉ aggregate static
  factory, không chứa build implementation cụ thể.
- [ ] Thêm assertion test: mọi factory đã đăng ký có đúng một descriptor cùng adapter ID và
  `ProviderType`; mọi descriptor active có typed config parser/factory tương ứng.

### 6. Descriptor, inspection và diagnostic

- [ ] Mỗi `*/descriptor.rs` cung cấp descriptor immutable, static capability và bootstrap
  inspector (nếu adapter cần dynamic create options).
- [ ] Bootstrap inspection vẫn metadata-only: không cần provider key/runtime, không persist,
  resolve secret, gọi remote bằng credential hoặc hot-load model.
- [ ] Tách provider-facing diagnostic operation/validation seam vào
  `providers/diagnostics.rs` nếu việc di chuyển giúp adapter tự sở hữu diagnostic implementation.
- [ ] Giữ `ProviderDiagnosticService` tại `services/`; không đổi các invariant về workload-aware
  admission, Voice reservation, cleanup acknowledgement và quarantine.

### 7. Cập nhật caller và compatibility

- [ ] Cập nhật imports trong loader, `RuntimeCatalog`, worker và Admin handler theo public
  re-export mới; không thay HTTP response hay error taxonomy.
- [ ] Giữ `providers::ProviderDescriptor`, `ProviderAdapterRegistry`, `ProviderRegistry`,
  `compiled_provider_adapter_registry()` và `compiled_provider_registry()` hoạt động ở path cũ.
- [ ] Xác minh Provider Test API tiếp tục dùng immutable provider `key`, runtime đã load và không
  hot-load hoặc ghi history.
- [ ] Không refactor VAD/Vision trong scope này. Nếu cần đưa chúng vào layout chung, cập nhật guide
  thiết kế trước và thêm directory/descriptor contract tương đương trong thay đổi riêng.

### 8. Test và completion gate

- [ ] Chuyển test unit cùng module sang `tests.rs` hoặc `tests/` cạnh module mới; không để
  `mod.rs`/`provider.rs` tiếp tục phình vì test.
- [ ] Chạy test descriptor/config consistency và registry/factory alignment.
- [ ] Chạy test Admin descriptor, bootstrap discovery và Provider Test API public boundary.
- [ ] Chạy `cargo fmt --check`.
- [ ] Chạy `git diff --check`.
- [ ] Chạy `cargo test --workspace`.
- [ ] So sánh JSON descriptor và route response trước/sau refactor cho adapter active.

## Thứ tự commit khuyến nghị

1. Tách common types và giữ re-export không đổi.
2. Tách từng adapter (ASR, TTS, LLM) thành commit độc lập, không thay behavior.
3. Đổi registry factory/descriptor và thêm alignment tests.
4. Tách diagnostics seam nếu còn coupling sau ba adapter.
5. Xóa compatibility re-export nội bộ chỉ khi toàn bộ caller đã chuyển và có commit riêng.

Không trộn refactor cấu trúc với migration schema/provider config hoặc thay đổi Voice semantics.
