# Provider-Owned Model Assets — Kế hoạch triển khai

Kế hoạch này là kế hoạch triển khai cho `docs/provider_owned_model_assets_refactor_guide.md`.
Guide là spec; file này là bản đồ thay đổi đã đối chiếu với code hiện tại.

Trạng thái: **đã triển khai xong** trên nhánh `refactor`.

## Ghi chú thực thi

Ba quyết địch ở mục 1 đã được giải quyết như sau:

1. `RuntimePhase::ArtifactVerify` — **giữ nguyên enum và key JSON** (`artifact_verify`),
   chỉ đổi doc từ "SHA-256 verification" sang "resolve path". `MaterializationTimings.artifact_verify`
   không còn được ghi giá trị nữa. Đổi shape sẽ phá consumer của report đã lưu.
2. `BenchmarkErrorCategory::ModelPreparation` — **giữ**, wire value `"model_preparation"`
   không đổi (ADR 0047 đã chốt). `comparison_qualified` bị **xoá** vì nó chỉ tồn tại để
   diễn đạt offline mode; `model_preparation_ms` giữ và nay đo thời gian `ensure_assets()`.
3. `measured_manifest_sha256` — **xoá** khỏi `ProviderRuntimeConfig` như guide chỉ định.
   Test gate receipt cũ đã bị thay bằng test gate revision của asset manager.

Hai điểm lệch so với kế hoạch, đều do code thật:

- Guide §4 gợi ý `providers/vad/silero/assets.rs`. Repo không có thư mục `vad/silero/`, nên
  `silero_descriptor.rs` và `silero_onnx.rs` đã được `git mv` vào `vad/silero/` theo đúng
  layout guide mô tả, thay vì đặt `assets.rs` cạnh module cũ.
- Guide §12 định nghĩa `ZeroTtsAssets` như struct mới. `ZeroTtsArtifacts<'a>` đã tồn tại, nên
  struct mới thay thế nó và struct cũ bị xoá.
- Kokoro cần một type `Voicepack` riêng thay vì dùng `VoiceAsset`, vì file nó đọc (`.bin`) là
  file nó tạo ra (`.pt`), không phải file tải về nguyên vẹn.

Hai việc phát sinh trong lúc code, không nằm trong kế hoạch gốc:

- **Có lúc đã bỏ rồi khôi phục việc materialize provider mặc định ở `managed_startup`.** Ban
  đầu tôi hiểu §13 là "startup không được chạm model" nên đã bỏ vòng acquire, dẫn tới request
  đầu tiên phải chờ tải. Yêu cầu thật của operator là startup phải có model sẵn, nên vòng
  acquire đã quay lại, chỉ khác ở chỗ nó gọi `ensure_assets()` theo provider thay vì verify
  checksum. Checklist §33 "Generic server startup không tải model" được hiểu lại là "không
  scan, không verify toàn bộ model", không phải "không tải gì cả" — và đây là điểm guide
  §13/§34 diễn đạt mơ hồ.
- Phần chọn provider được nạp trước đã tách thành `startup_providers()` để test được độc lập
  với model, vì một test "startup materialize default provider" không thể chạy hermetic.
- **`prepare_artifacts` chỉ chạy cho acquisition speculative** (`registry.rs`), nên acquisition
  tường minh của provider mặc định gọi thẳng `build` mà không tải gì. Trước refactor
  `prepare_artifacts` chỉ là một cache lookup nên bỏ qua cũng vô hại; sau khi nó trở thành nơi
  tải model, điều đó là bug. Đã sửa: mọi acquisition đều đi qua `prepare_artifacts`.

Đã kiểm chứng bằng cold start thật: cwd không có `models/`, chạy server → tải 5 file (85 MB)
cho silero + gipformer, materialize đủ 4 provider mặc định, rồi mới bind. Chạy lần hai →
5 asset `reusing`, 0 download.
- **`load_local` (unmanaged path) phải `ensure_assets()` trước khi build.** Path này không có
  Provider Runtime Manager, nên không có chỗ nào khác để tải. Đây là lý do `ensure_assets` tồn
  tại ở cả hai path.
- `models/` trong `.gitignore` đổi từ `/models/**` sang `models/`, vì model install là output
  chứ không phải source, và test có thể tạo ra nó trong crate directory.

---

## 1. Đối chiếu spec với codebase

Guide viết theo spec mong muốn, không theo trạng thái repo. Kết quả đối chiếu:

| Mục | Spec (guide) | Thực tế repo | Đánh giá |
|---|---|---|---|
| `models/manifest.toml` | xóa | tồn tại, 390 dòng, **5 model / 51 artifact** | khớp |
| `.installed/<fingerprint>` | xóa | tồn tại, **7 fingerprint dir** đã tải về | khớp |
| SHA-256 verify | xóa | `models.rs:572 hash_file`, `verify_path`, `Artifact.sha256/source_sha256` | khớp |
| Offline mode | xóa | `config/mod.rs:546 ModelStoreConfig.offline`, ép `true` ở 4 chỗ | khớp |
| ZeroTTS descriptor voices | trùng `assets.rs` | `zerotts/descriptor.rs:26-75` đã có đúng 8 id, khớp 8 role `voice_*` trong manifest | khớp |
| `ZeroTtsAssets` struct (guide §12) | tạo mới | **đã tồn tại**: `src/providers/tts/mod.rs:256 ZeroTtsArtifacts<'a>` (thiếu `null_voice`, `codec_license`) | tái dùng, không tạo mới |
| `ProviderAssetManager` (guide §15) | thêm vào registration | `registry.rs:8 ProviderAdapterRegistration` chỉ có **2 field**, không có field asset | cần thêm |
| `models/SpeakerRecognition/` (guide §2) | có trong cây thư mục | **không tồn tại provider nào** dùng speaker recognition | bỏ khỏi scope |
| `sha2` dep | (không nói) | **phải giữ**: `services/provider_runtime/plan.rs:137` dùng `Sha256::digest` cho `ResourceKey` | giữ `sha2` |
| `zip` dep | (không nói) | chỉ dùng ở `models/kokoro.rs:22` (transform voicepack) | chuyển sang `kokoro_vi/assets.rs` |
| `verify_installed` + bin offline preflight | xóa | `bin/phase5-offline-preflight.rs` (148 dòng) + `scripts/test-phase5-offline-preflight.sh` **mất hết ý nghĩa** | xóa cả 2 |
| `RuntimePhase::ArtifactVerify` | (không nói) | `metrics.rs:24` mô tả "**SHA-256 verification**", là phần tử JSON export | cần quyết định |
| `BenchmarkErrorCategory::ModelPreparation` | (không nói) | `benchmark/error.rs:5`, wire value `"model_preparation"` ổn định (ADR 0047) | cần quyết định |

### 5 model trong manifest (nguồn URL phải chuyển vào `assets.rs`)

| identity | adapter | số artifact | transform đặc biệt |
|---|---|---|---|
| `silero_vad_v5` | `silero_onnx` | 1 | — |
| `zipformer_vi_streaming` | `zipformer_sherpa` | 4 | `sentencepiece_tokens_v1` (bpe.model → tokens.txt) |
| `gipformer15_vi_int8` | `gipformer_sherpa_offline` | 4 | — |
| `zerotts_default` | `zerotts_onnx` | 21 (13 core + 8 voice) | — |
| `kokoro_vi_contextbox` | `kokoro_vi_onnx` | 16 (model + config + 14 voicepack) | `kokoro_voicepack_v1` (voicepack.pt → .bin) |

### Ba quyết định cần chốt trước khi code

1. **`ArtifactVerify` phase**: giữ (đổi nghĩa thành "resolve path") hay bỏ khỏi `RuntimePhase`? Bỏ sẽ đổi cả key JSON export lẫn `BOUNDS` array length (`metrics.rs:44`).
2. **`BenchmarkErrorCategory::ModelPreparation`**: giữ làm category cho lỗi asset (đổi doc) hay xóa? Xóa sẽ phá schema report mà ADR 0047 đã chốt.
3. **`measured_manifest_sha256`** (guide §22): xóa hẳn field khỏi `ProviderRuntimeConfig` (guide nói vậy) — nhưng `config.toml:262` đang có giá trị thật và `provider_materializer.rs:475` có test gate nó. Xác nhận xóa và rewrite test thành gate `asset revision constant + onnx runtime identity`.

---

## 2. Danh sách file cần update

### 2.1 Tạo mới

| File | Nội dung |
|---|---|
| `src/providers/assets.rs` | `MODELS_ROOT`, `model_path()`, `Asset`, `VoiceAsset`, `AssetError`, `is_ready()`, `download_atomic()`, per-asset striped lock, `ProviderAssetManager` trait |
| `src/providers/tts/zerotts/assets.rs` | `MODEL_DIR`, `MODEL_REVISION`, 13 `CORE_ASSETS`, 8 `VOICES`, `ensure_assets()`, `resolve_assets()` |
| `src/providers/tts/kokoro_vi/assets.rs` | `MODEL_DIR`, `MODEL_REVISION`, model + config + 14 voicepack, `voicepack_v1` transform inline |
| `src/providers/asr/gipformer/assets.rs` | 4 asset, identity |
| `src/providers/asr/zipformer/assets.rs` | 4 asset, `sentencepiece_tokens_v1` transform inline |
| `src/providers/vad/assets.rs` (hoặc `silero/assets.rs`) | 1 asset `silero_vad.onnx` |
| `docs/adr/0076-provider-owned-model-assets.md` | ADR mới (guide §30) |

Guide §4 gợi ý `providers/vad/silero/assets.rs`. Code hiện tại **không có** thư mục `providers/vad/silero/` — VAD là `vad/mod.rs` + `vad/silero_descriptor.rs` + `vad/silero_onnx.rs`. Phải chọn: tách thư mục `vad/silero/` hay đặt `vad/assets.rs`. Khuyến nghị: tách thư mục `vad/silero/` cho khớp guide.

### 2.2 Sửa — lõi model framework (xoá gần hết)

| File | Thay đổi |
|---|---|
| `src/models.rs` (703 dòng) | **xoá toàn bộ**: `Manifest`, `Model`, `Artifact`, `ResolvedModel`, `ModelError`, `ModelAcquirer`, `ModelPreparation(Config)`, `prepare*`, `verify_installed`, `model_fingerprint`, `fingerprint_model`, `verify_path`, `verifies`, `hash_file`, `validate_relative_path`, `safe_install_path`, `safe_existing_install_path`, `transform`, `sentencepiece_tokens`, `read_varint`, `require_acknowledgement` |
| `src/models/prepared.rs` (145) | **xoá cả file** — `PreparedModelCatalog` key theo `model_fingerprint` |
| `src/models/startup.rs` (142) | **xoá cả file** — `prepare_startup()` |
| `src/models/startup/tests.rs` | **xoá** |
| `src/models/acquisition.rs` (116) | giữ `HttpModelAcquirer` (dùng raw `TcpStream`, không phụ thuộc `reqwest`); đổi sang `acquire(remote, dest) -> AssetError`; giữ retry/no-append semantics cho `tests/model_download.rs` |
| `src/models/kokoro.rs` (87) | **di chuyển** sang `providers/tts/kokoro_vi/assets.rs` (chứa `voicepack_v1` transform + `zip` dep usage) |
| `src/models/kokoro_tensor_v1.pkl` | giữ, move cùng file |

Sau khi xong, `src/models/` chỉ còn downloader. Guide §18 cho phép đổi thành `src/assets/`. Khuyến nghị: gộp thành `src/assets.rs` + `src/assets/` và xóa hẳn module `models` để không còn abstraction cũ.

### 2.3 Sửa — provider

| File:line | Thay đổi |
|---|---|
| `src/providers/registry.rs:8-12` | thêm field `assets: Option<&'static dyn ProviderAssetManager>` vào `ProviderAdapterRegistration` |
| `src/providers/registry.rs:57-65` | 7 `REGISTRATION` — ZeroTTS/Kokoro/Gipformer/Zipformer/Silero gắn `assets`, OpenAI/ChillAudio để `None` |
| `src/providers/tts/zerotts/mod.rs` | thêm `pub mod assets;` |
| `src/providers/tts/zerotts/descriptor.rs:158-161` | `REGISTRATION` gắn `assets: Some(&ASSET_MANAGER)`; `VOICES` (`:26-75`) chuyển sang lấy từ `assets::VOICES` để một nguồn duy nhất |
| `src/providers/tts/kokoro_vi/mod.rs` | thêm `pub mod assets;`; `descriptor.rs` REGISTRATION |
| `src/providers/asr/gipformer/mod.rs`, `zipformer/mod.rs` | thêm `pub mod assets;` |
| `src/providers/vad/mod.rs` | khai báo module silero mới |
| `src/providers/tts/mod.rs:256-269` | `ZeroTtsArtifacts<'a>` → lấy từ `zerotts::assets::resolve_assets()`; bỏ `voices_index` khỏi download list nếu runtime vẫn cần thì giữ |
| `src/providers/factory_registry.rs` | |
| `:174-209` SileroOnnxFactory | `build(..., model: &ResolvedModel)` → `build(..., assets: &SileroAssets)`; `required(model,"vad")` → `assets.model` |
| `:318-351` ZeroTts | `ZEROTTS_REQUIRED_ARTIFACT_ROLES` (`:476-490`) xóa; `.artifact("...")` (13 call) → field của `ZeroTtsAssets`; dynamic `format!("voice_{}", id)` (`:327`) → `assets.voices[id]` |
| `:428` Kokoro | `KOKORO_VI_REQUIRED_ARTIFACT_ROLES` xóa; dynamic `format!("voicepack_{}", voice)` (`:411`) → map |
| `:492-540` Zipformer | 4 `required(...)` → field |
| `:542-609` Gipformer | 4 `required(...)` → field |
| `:665-670` `required()` | xóa |
| `:642-663` `validate_model_adapter` | đổi sang kiểm tra adapter + revision constant, không dùng `model.identity()` |
| `:672-682` `local_model_identity` | giữ (dùng cho ResourceKey identity + label), nhưng không còn là manifest key |
| `:683-704` `effective_local_config` | `object.insert("model", ...)` — quyết định có còn giữ `model` trong provider config hay bỏ (guide §20 nói ZeroTTS config không có `model`) |
| `src/providers/factory_registry/tests.rs` (134) | 4 test dùng `ResolvedModel::for_test` → rewrite dùng struct asset thật |
| `src/providers/loader.rs:111-118, 156-163, 219-231` | `models::prepare(...)` → `factory.ensure_assets()` / `resolve_assets()` |
| `src/providers/database_loader.rs:16` | xóa import `models::prepare_immutable as prepare` |
| `:255, :270` | xóa 2 assignment `offline = true` |
| `:284-304` `selected_model()` | xóa; thay bằng `assets.ensure_assets()` + `resolve_assets()` |
| `:261-282` `materialize_provider_from_artifacts` | đổi tham số `Option<&ResolvedModel>` → `Option<ProviderAssets>` hoặc bỏ hẳn |
| `src/providers/error.rs:20` | `Manifest(#[from] crate::models::ModelError)` → `Assets(#[from] AssetError)` |
| `src/providers/vad/silero_onnx.rs:22-43` | `load(model: String, ...)` → nhận `PathBuf` từ `SileroAssets` |
| `src/providers/tts/kokoro_vi/mod.rs:25-29, 48-68` | `KokoroViArtifacts<'a>` → lấy từ `kokoro_vi::assets` |
| `src/providers/tts/zerotts/runtime/contract.rs` | `GraphPaths`/`CodecPaths` nhận `PathBuf` từ `ZeroTtsAssets` thay vì `&Path` |

### 2.4 Sửa — runtime manager

| File:line | Thay đổi |
|---|---|
| `src/services/provider_runtime/factory/mod.rs` | |
| `:28-30` `FactoryDiagnostics` | `model_preparations`/`model_cache_hits` → bỏ hoặc đổi nghĩa thành `asset_ensures` |
| `:38-43` struct fields | xóa `qualified_manifest_fingerprint`, `prepared_models` |
| `:64-82` | xóa nhánh `manifest_fingerprint` + so sánh `measured_manifest_sha256` |
| `:121, 128, 149` `verify_qualified_manifest()` | xóa 3 call |
| `:124-147` `prepare_artifacts()` | dùng `registration.assets.ensure_assets()` (guide §15) |
| `:251-270` `build()` | xóa nhánh `prepared_models.resolve(...)`; chỉ `resolve_assets()` |
| `:492-499` `verify_qualified_manifest()` | xóa |
| `:501-540` `resource_key_with_fingerprint()` | bỏ `model_fingerprint(...)`; dùng `adapter + MODEL_REVISION + onnx identity + threads` (guide §21) |
| `:543-545` `manifest_fingerprint()` | xóa |
| `:547-570` `execution_file_fingerprint()` | **giữ** (onnx library + g2p executable — không phải model artifact) |
| `:572-584` `kokoro_g2p_fingerprint()` | giữ |
| `src/services/provider_runtime/plan.rs:119-154` | `PhysicalResourceIdentity.artifact_fingerprint: String` → `asset_revision: &'static str`. Giữ `sha2` cho `ResourceKey` |
| `src/services/provider_runtime/metrics.rs:22-25, 44, 55, 110-111` | `ArtifactVerify` — theo quyết định #1 |
| `src/services/provider_runtime/factory/tests.rs` (95) | cập nhật |

### 2.5 Sửa — app / startup

| File:line | Thay đổi |
|---|---|
| `src/app/mod.rs:195-206` | xóa `spawn_blocking(prepare_startup)` + 3 clone |
| `src/app/mod.rs:210-214` | xóa `startup_config` + `offline = true` + comment |

Sau đó startup chỉ còn: config → DB → migrations → registry → `ProviderRuntimeManager` → bind.

### 2.6 Sửa — config

| File:line | Thay đổi |
|---|---|
| `src/config/mod.rs:518-529` | `DeploymentConfig` xóa `model_manifest`, `model_acknowledgements`, `models` |
| `:531-540` | `impl Default` tương ứng |
| `:542-562` | xóa `ModelStoreConfig` + `impl Default` |
| `:610-616` | xóa `ModelAcknowledgement` |
| `:1126-1135` | `ProviderRuntimeConfig` xóa `measured_manifest_sha256` (`:1132`) |
| `src/config/defaults.rs:181-186` | xóa `default_manifest_path()`, `default_models_root()` |
| `src/config/validation.rs:699-721` | xóa validate `models.sources` |
| `src/config/validation.rs:722-729` | xóa check `model_manifest` / `models.root` non-empty |
| `src/config/validation.rs:738-740` | xóa `mod model_sources_tests` |
| `src/config/model_sources_tests.rs` (56) | **xoá cả file** |
| `config.example.toml:179-188` | xóa `model_manifest`, `[deployment.models]` |
| `config.example.toml:204-222` | xóa 4 `[[deployment.model_acknowledgements]]` |
| `config.example.toml:252-255, 267` | xóa prose về manifest receipt + `measured_manifest_sha256` |
| `config.toml:168-174, 179-202, 262` | tương tự (file local, gitignored) |
| `.gitignore:4-5` | `/models/**` + `!/models/manifest.toml` → `/models/**` |

### 2.7 Xóa

| File | Lý do |
|---|---|
| `models/manifest.toml` | guide §17 |
| `models/.installed/` (7 dir) | guide §14 — không còn `.installed/` |
| `src/bin/phase5-offline-preflight.rs` (148) | chỉ làm `verify_installed` |
| `scripts/test-phase5-offline-preflight.sh` (5) | chạy bin trên |
| `tests/model_preparation.rs` (910) | 19/19 test bảo vệ behavior bị xóa |
| `src/providers/factory_registry/tests.rs` | 4 test dùng `ResolvedModel::for_test` — hoặc rewrite |

### 2.8 Test — sửa / viết mới

| File | Hành động |
|---|---|
| `tests/model_preparation.rs` | xóa; thay bằng `tests/provider_assets.rs` mới theo guide §27 (10 test) |
| `tests/zerotts_artifact_preflight.rs` (332) | `:33, 120, 147, 190, 224` rewrite; giữ `:176` (`warmup_pcm_...`) |
| `tests/provider_registry.rs:197, 242` | `every_advertised_local_voice_has_a_pinned_preparation_artifact` → so với `assets::VOICES`; `native_transducers_accept_every_advertised_decoding_method` → dùng `assets::ensure_assets()` |
| `tests/provider_materializer.rs` | `:475` receipt test → xóa/rewrite; `:542` bỏ `measured_manifest_sha256` khỏi fixture; `:572` example config; `:641 zerotts_config()`; `:850 synthetic_zerotts_root()` → `resolve_assets()` fixtures; `:890 one_materialization_prepares_its_immutable_model_exactly_once` → viết lại thành "second load không download" |
| `tests/phase4_reference_gate.rs:96-138 real_tts()` | bỏ `ModelAcknowledgement`, `ModelStoreConfig`, `models::prepare` → `zerotts::assets::resolve_assets()` |
| `tests/model_download.rs` (84) | giữ nếu `HttpModelAcquirer` giữ trait `acquire(remote, dest)`; 3 test (`:47, 61, 74`) |
| `tests/provider_load_plan.rs` (390) | kiểm tra lại — có thể dựa `on prepare_startup` |

### 2.9 Bin khác

| File:line | Thay đổi |
|---|---|
| `src/bin/provider-bench.rs:6, 117-132, 136, 157, 161, 316-319` | `--require-local-models` và `verify_installed` → `resolve_assets()`; `BenchmarkErrorCategory::ModelPreparation` theo quyết định #2 |
| `src/bin/provider-bench-av.rs:13, 173-180, 238-245` | `models::prepare` → `ensure_assets()` |
| `src/bin/provider-runtime-bench.rs:53, 71, 95, 98, 103` | `:53` xóa `offline = true`; `:95/:98/:103` theo quyết định #1 |
| `src/bin/zerotts-core-check.rs` | không đổi (env-var path) |
| `scripts/test-phase4-reference-gate.sh:4, 9-21, 26-39` | path `models/zerotts` → `models/TTS/zerotts` nếu đổi layout; `ZEROTTS_*` env list phải regenerate từ `assets.rs` |
| `scripts/test-phase5-reference-gate.sh:11` | bỏ step offline preflight |

### 2.10 Docs

| File | Hành động |
|---|---|
| `docs/adr/0044-pinned-startup-model-preparation.md` | đánh dấu **Superseded** (guide §29) |
| `docs/adr/0076-provider-owned-model-assets.md` | tạo mới (guide §30) |
| `docs/adr/0075-startup-artifacts-and-database-directories.md` | cập nhật phần `prepared://` / manifest revision / offline |
| `docs/adr/0043, 0027, 0047, 0071` | sửa tham chiếu |
| `docs/02-codebase.md:19` | mô tả module `models` |
| `docs/03-module-contracts.md:67` | `Model Preparation`, `ResolvedModel` |
| `docs/04-configuration.md:53-55, 246, 248, 249, 265` | xóa `[deployment.models]`, mô tả checksum |
| `docs/06-implementation-plan.md:47, 65, 75` | Phase description |
| `docs/kokoro-vi-provider.md:33-48` | checksum, `deployment.models.sources`, offline |
| `docs/zerotts-runtime-optimization-guide.md` (16 chỗ) | `prepare_immutable()` mô tả |
| `docs/provider-runtime-manager-three-phase-guide.md` (7 chỗ) | fingerprint, `src/models.rs` |
| `docs/provider-runtime-config-refactor-implementation.md:5, 60, 70, 73, 87, 96` | SHA, offline, receipt |
| `docs/provider-runtime-manager-qualification.md:45` | "model preparation" |
| `docs/performance_tester_rust_guide.md` (14 chỗ) | `model_preparation_ms` |
| `README.md:43-49, 67-70` | mô tả Model Preparation |
| `CONTEXT.md:188, 196-197, 249, 539, 543` | domain doc — **bắt buộc cập nhật** |
| `docs/flow.md`, `docs/flows/*`, `docs/testing/*`, `AGENTS.md` | không có hit — không đổi |

### 2.11 Cargo

| Dep | Hành động |
|---|---|
| `sha2` | **GIỮ** — `services/provider_runtime/plan.rs:137` |
| `zip` | giữ **nếu** `kokoro_v1` transform move sang `kokoro_vi/assets.rs`; xóa nếu không |
| `toml` | giữ — config parser chính |
| `reqwest` | giữ — External MCP |

---

## 3. Plan triển khai

Mỗi phase kết thúc bằng `cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace` xanh, và có thể `git revert` độc lập.

### Phase 1 — Asset foundation (không xóa gì cũ)

Thêm mới, chưa ai dùng:

1. `src/providers/assets.rs`
   - `pub const MODELS_ROOT: &str = "models";`
   - `pub fn model_path(kind: &str, provider: &str) -> PathBuf`
   - `pub struct Asset { path, url }`, `pub struct VoiceAsset { id, name, path, url }`
   - `pub enum AssetError { Io, Download, InvalidDownload, Transform, Missing }`
   - `pub fn is_ready(path: &Path) -> bool` — chỉ `metadata.is_file() && len() > 0`
   - `fn asset_lock(path) -> MutexGuard` — port từ `models.rs:254-282` (striped 64)
   - `pub fn download_atomic(url, target) -> Result<PathBuf, AssetError>` — `.part` → size check → rename
   - `pub trait ProviderAssetManager: Send + Sync { fn ensure_assets(&self) -> Result<(), AssetError>; }`
2. `src/providers/registry.rs:8` thêm field `assets: Option<&'static dyn ProviderAssetManager>`; 7 `REGISTRATION` set `None`.
3. `src/models/acquisition.rs` — sửa `ModelError` → `AssetError` (giữ `acquire(remote, dest)` + retry). `tests/model_download.rs` vẫn xanh.
4. Test mới cho foundation: reuse khi có file, download khi thiếu, zero-byte re-download, interrupted không tạo final file, `.part` cleanup, concurrent ensure một lần.

Exit: build xanh, `tests/model_download.rs` xanh, không có call site mới.

### Phase 2 — `assets.rs` cho ZeroTTS (pilot)

1. `src/providers/tts/zerotts/assets.rs`
   - `MODEL_DIR = "models/TTS/zerotts"`, `MODEL_REVISION = "c2bfbd67..."`
   - 13 `CORE_ASSETS` — copy URL từ `models/manifest.toml:104-197`
   - 8 `VOICES` — URL từ `manifest.toml:139, 203-253`
   - `ZeroTtsAssets` struct (di chuyển từ `src/providers/tts/mod.rs:256-269`, thêm `null_voice`/`codec_license` nếu giữ trong CORE_ASSETS)
   - `ensure_assets()` = ensure CORE_ASSETS + ensure **ALL** VOICES
   - `resolve_assets()` = chỉ check `is_ready` + trả struct `PathBuf`
2. `descriptor.rs` — `VOICES` lấy từ `assets::VOICES` (một nguồn). `REGISTRATION.assets = Some(&ASSET_MANAGER)`.
3. `mod.rs` — `pub mod assets;`
4. `factory_registry.rs:318-351` — nhận `ZeroTtsAssets` thay `&ResolvedModel`; xóa `ZEROTTS_REQUIRED_ARTIFACT_ROLES`.
5. Test: guide §27.6 (đảm bảo đủ 8 voice), §27.7 (descriptor voice IDs == asset voice IDs).

Exit: `zerotts::assets::ensure_assets()` tải đủ 8 voice vào model folder rỗng. Chưa xóa manifest.

### Phase 3 — `assets.rs` cho provider còn lại

Theo thứ tự độc lập, mỗi cái một commit:

1. **Silero** — `providers/vad/silero/` (tách từ `vad/silero_onnx.rs` + `vad/silero_descriptor.rs`), `assets.rs` 1 file. `factory_registry.rs:202` dùng path.
2. **Gipformer** — `providers/asr/gipformer/assets.rs`, 4 asset. `factory_registry.rs:596-600`.
3. **Zipformer** — `providers/asr/zipformer/assets.rs`, 4 asset + `sentencepiece_tokens_v1` transform inline (port `models.rs:651-703`). `factory_registry.rs:525-528`.
4. **Kokoro VI** — `providers/tts/kokoro_vi/assets.rs`, 16 asset + `voicepack_v1` transform (move `src/models/kokoro.rs`, giữ `zip`). `factory_registry.rs:408-420`.
5. Remote (`openai`, `chillaudio_ws`, `openai_vision`) — `assets: None`, không làm gì.

Mỗi provider: `factory_registry.rs` build nhận struct asset riêng; xóa `required()` dần.

Exit: 5 provider local đều có `assets.rs`, không còn `.artifact(role)` nào.

### Phase 4 — Chuyển Runtime Manager

1. `FactoryMaterializer::prepare_artifacts()` → `registration.assets.ensure_assets()` (guide §15, không `match` theo adapter).
2. `build()` → chỉ `resolve_assets()` + construct. Xóa `PreparedModelCatalog`.
3. `resource_key_with_fingerprint` → `resource_key`, bỏ `model_fingerprint`. `plan.rs:120-154` đổi `artifact_fingerprint` → `asset_revision: &'static str`.
4. Xóa `verify_qualified_manifest()` + `qualified_manifest_fingerprint` + `manifest_fingerprint()`.
5. Áp dụng quyết định #1 (`ArtifactVerify`) và #3 (`measured_manifest_sha256` + test `provider_materializer.rs:475`).
6. `database_loader.rs` — xóa `selected_model()`, 2 assignment `offline = true`, đổi signature `materialize_provider_from_artifacts`.
7. `loader.rs:111-231` — `prepare(...)` → `ensure_assets()` + `resolve_assets()`.
8. `FactoryDiagnostics` — đổi counter.

Exit: `cargo test --workspace` xanh (trừ test đã đánh dấu xóa ở Phase 6), không còn `prepare_immutable` trong `src/`.

### Phase 5 — Xóa startup model preparation

1. `app/mod.rs:195-206` — xóa `spawn_blocking(prepare_startup)`.
2. `app/mod.rs:210-214` — xóa `startup_config` clone + `offline = true`.
3. Xóa `src/models/startup.rs` + `src/models/startup/tests.rs`.
4. Test guide §27.8: start server với model folder không tồn tại, không preload → bind OK, không log `preparing startup model artifacts`.
5. Kiểm tra `preload=true` vẫn tải qua `ProviderRuntimeManager` preload (guide §14).

Exit: server start với `models/` rỗng hoặc không có.

### Phase 6 — Xóa framework + config + manifest

1. Xóa `models/manifest.toml`, `models/.installed/`.
2. Xóa `src/models.rs` (giữ downloader), `src/models/prepared.rs`, `src/models/kokoro.rs` (đã move).
3. Gộp downloader còn lại: `src/models/acquisition.rs` → `src/assets/download.rs`; xóa hẳn module `models`.
4. `config/mod.rs`, `config/defaults.rs`, `config/validation.rs` — xóa field đã liệt kê.
5. Xóa `src/config/model_sources_tests.rs`.
6. `config.example.toml`, `config.toml`, `.gitignore`.
7. Xóa `src/bin/phase5-offline-preflight.rs` + `scripts/test-phase5-offline-preflight.sh`; sửa `scripts/test-phase5-reference-gate.sh`.
8. `Cargo.toml` — bỏ `zip` nếu không dùng.
9. `benchmark/error.rs` + `bin/provider-bench*.rs` — áp dụng quyết định #2.

Exit: `rg "model_manifest|manifest\.toml|source_sha256|prepare_immutable|prepare_startup|model_acknowledgements|models\.offline|models\.root|models\.sources|measured_manifest_sha256"` trong `src/` và `tests/` trả 0 kết quả (trừ chính guide/plan này).

### Phase 7 — Tests + docs

1. Viết `tests/provider_assets.rs` mới theo guide §27.1-27.10.
2. Rewrite `tests/zerotts_artifact_preflight.rs`, `tests/provider_registry.rs`, `tests/provider_materializer.rs`, `tests/phase4_reference_gate.rs`.
3. Xóa `tests/model_preparation.rs`.
4. `scripts/test-phase4-reference-gate.sh` — regenerate path list từ `assets.rs`.
5. ADR: 0044 → Superseded; tạo `docs/adr/0076-provider-owned-model-assets.md`.
6. Sửa docs theo bảng §2.10. `CONTEXT.md` bắt buộc.
7. Final sweep: chạy lệnh `rg` ở guide §29 trên toàn repo.

Exit: guide §33 — tất cả 22 checkbox xanh.

---

## 4. Rủi ro

| Rủi ro | Ảnh hưởng | Giảm thiểu |
|---|---|---|
| ONNX external data của ZeroTTS codec phải là sibling trên disk (`codec.rs:397-417` chỉ so tên file + `is_file()`; ONNX tự resolve lúc `commit_from_file`) | Đổi layout làm hỏng codec runtime | Giữ đúng relative path `onnx/codec/*` trong `assets.rs`; test 27.6 phải assert file sibling |
| `resolve_assets()` + `ensure_assets()` bị gọi lặp lại ở `prepare_artifacts` và `build` | Trái guide §16 | `build` chỉ gọi `resolve_assets()` (metadata-only) |
| 8 voice `.npz` ZeroTTS + 14 voicepack Kokoro phải tải hết | Startup lần đầu chậm, disk tăng | Chấp nhận (guide §7, §32). `preload=false` mặc định |
| `sha256` vẫn cần cho `ResourceKey` | Dễ xóa nhầm | Giữ `plan.rs:137`; không đụng `Cargo.toml` sha2 |
| `scripts/test-phase4-reference-gate.sh` đọc layout cũ `models/zerotts/**` | CI fail | Regenerate từ `assets.rs` ở Phase 7 |
| Guide §2 nhắc `models/SpeakerRecognition/` nhưng provider không tồn tại | Scope creep | Bỏ khỏi cây thư mục trong ADR mới |
| `benchmark/error.rs` wire value + `metrics.rs` JSON key đã được ADR 0047 chốt | Breaking report consumers | Chốt quyết định #1/#2 trước Phase 4 |

---

## 5. Definition of Done

Theo guide §33, tất cả phải xanh:

- [ ] Không `models/manifest.toml`, không `models/.installed/`
- [ ] Không SHA-256 cho model artifact (sha2 vẫn còn cho ResourceKey)
- [ ] Không `prepare_startup()`, không scan model khi startup
- [ ] Config không có `model_manifest`, `models.root`, `models.offline`, `models.sources`, `model_acknowledgements`, `measured_manifest_sha256`
- [ ] 5 provider local có `assets.rs`; URL nằm trong source
- [ ] `is_ready` = regular file + `size > 0`
- [ ] Download qua `.part` + atomic rename + per-asset lock
- [ ] ZeroTTS đảm bảo **toàn bộ** 8 voice; descriptor và asset catalog đồng bộ
- [ ] `build()` không download/checksum
- [ ] `FactoryMaterializer` không `match snapshot.adapter`
- [ ] Admin API không expose model path/URL
- [ ] 10 test guide §27 xanh; test cũ của behavior đã xóa đã bị xóa
- [ ] ADR 0044 Superseded, ADR 0076 tạo mới, docs + `CONTEXT.md` cập nhật