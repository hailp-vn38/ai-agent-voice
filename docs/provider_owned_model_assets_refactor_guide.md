# Hướng dẫn refactor Model Assets theo Provider

## 1. Mục tiêu

Đơn giản hóa hoàn toàn cơ chế quản lý và tải model trong `ai-agent-voice`.

Thiết kế mới phải tuân theo các nguyên tắc:

- Mỗi provider tự khai báo và tự quản lý các file model mà nó cần.
- Danh sách URL model nằm trong `assets.rs` của provider.
- Không dùng `models/manifest.toml`.
- Không kiểm tra SHA-256.
- Không có `model fingerprint`.
- Không có immutable model tree `.installed/<fingerprint>/...`.
- Không có `offline` mode.
- Không cho user cấu hình model folder.
- Không cho user cấu hình URL model.
- Không cho user cấu hình model path.
- Không scan hoặc verify toàn bộ model khi server startup.
- File đã tồn tại và có kích thước > 0 thì dùng ngay.
- File chưa tồn tại thì tự động tải.
- Download phải dùng file tạm `.part` và atomic rename để tránh coi file tải dở là file hợp lệ.
- Provider có voices thì tải toàn bộ voice khi provider assets được chuẩn bị.
- Provider config chỉ chứa các thuộc tính provider mà user thực sự cần chỉnh.

Thiết kế này ưu tiên sự đơn giản, startup nhanh và phù hợp với server homelab.

---

## 2. Cấu trúc thư mục model mới

Model root là cố định trong project/runtime và không expose qua config.

Cấu trúc mục tiêu:

```text
models/
├── TTS/
│   ├── zerotts/
│   │   ├── config.json
│   │   ├── tokenizer.json
│   │   ├── null_voice_emb.npy
│   │   ├── silence_frame.npy
│   │   ├── onnx/
│   │   │   ├── text_encoder.onnx
│   │   │   ├── prefix_step.onnx
│   │   │   ├── local_frame_decode.onnx
│   │   │   └── codec/
│   │   │       ├── moss_audio_tokenizer_decode_full.onnx
│   │   │       ├── moss_audio_tokenizer_decode_step.onnx
│   │   │       ├── moss_audio_tokenizer_decode_shared.data
│   │   │       ├── codec_browser_onnx_meta.json
│   │   │       └── LICENSE-Apache-2.0.txt
│   │   └── voices/
│   │       ├── maichi/
│   │       │   └── voice.npz
│   │       ├── baotrang/
│   │       │   └── voice.npz
│   │       ├── giahuy/
│   │       │   └── voice.npz
│   │       ├── hamy/
│   │       │   └── voice.npz
│   │       ├── huuduc/
│   │       │   └── voice.npz
│   │       ├── kimoanh/
│   │       │   └── voice.npz
│   │       ├── quangminh/
│   │       │   └── voice.npz
│   │       └── tiendat/
│   │           └── voice.npz
│   │
│   └── kokoro_vi/
│       └── ...
│
├── ASR/
│   ├── gipformer/
│   │   ├── encoder.onnx
│   │   ├── decoder.onnx
│   │   ├── joiner.onnx
│   │   └── tokens.txt
│   └── zipformer/
│       └── ...
│
├── VAD/
│   └── silero/
│       └── silero_vad.onnx
│
└── SpeakerRecognition/
    └── ...
```

Không tạo thêm `.installed/`.

Không copy model sang content-addressed tree.

Runtime dùng trực tiếp file trong các folder trên.

---

## 3. Model root là hằng số nội bộ

Không cho phép cấu hình:

```toml
[deployment.models]
root = "..."
offline = false
```

Không có `models.root`.

Không có `offline`.

Không có `sources`.

Không có `model_manifest`.

Model root nên là hằng số nội bộ, ví dụ:

```rust
pub const MODELS_ROOT: &str = "models";
```

Có thể đặt trong module chung:

```text
crates/voice-agent-server/src/providers/assets.rs
```

hoặc module tương đương.

Có thể cung cấp helper:

```rust
pub fn model_path(kind: &str, provider: &str) -> PathBuf {
    Path::new(MODELS_ROOT)
        .join(kind)
        .join(provider)
}
```

Ví dụ:

```rust
model_path("TTS", "zerotts")
```

trả về:

```text
models/TTS/zerotts
```

Không expose helper này ra Admin API.

---

## 4. Ownership của model chuyển về provider

Mỗi local provider phải có module:

```text
assets.rs
```

Ví dụ ZeroTTS:

```text
crates/voice-agent-server/src/providers/tts/zerotts/
├── assets.rs
├── config.rs
├── descriptor.rs
├── mod.rs
└── runtime/
```

Các provider khác:

```text
providers/tts/kokoro_vi/assets.rs
providers/asr/gipformer/assets.rs
providers/asr/zipformer/assets.rs
providers/vad/silero/assets.rs
```

`assets.rs` là authoritative source cho:

- base model URL;
- revision upstream;
- danh sách required files;
- relative paths;
- danh sách voices;
- URL voices;
- provider-specific transform nếu cần;
- hàm `ensure_assets()`.

Không đặt các thông tin này trong database.

Không đặt các thông tin này trong config user.

---

## 5. Asset API chung

Tạo abstraction tối thiểu, tránh xây lại một Model Preparation framework phức tạp.

Ví dụ:

```rust
pub struct Asset {
    pub path: &'static str,
    pub url: &'static str,
}

pub struct VoiceAsset {
    pub id: &'static str,
    pub name: &'static str,
    pub path: &'static str,
    pub url: &'static str,
}
```

Có thể thêm loại asset khác nếu provider thực sự cần, nhưng không thêm các field:

```text
sha256
source_sha256
license_acknowledgement
fingerprint
install_revision_receipt
```

Revision có thể giữ dưới dạng hằng số dùng để pin URL:

```rust
pub const MODEL_REVISION: &str =
    "c2bfbd67dc648cac455077333f7cf5c18a2e3bb4";
```

Mục đích của revision là tạo URL immutable từ upstream, không dùng để tạo runtime manifest/fingerprint.

---

## 6. ZeroTTS `assets.rs`

ZeroTTS phải chuyển toàn bộ declaration model hiện đang nằm trong `models/manifest.toml` vào:

```text
crates/voice-agent-server/src/providers/tts/zerotts/assets.rs
```

Ví dụ:

```rust
use std::path::{Path, PathBuf};

pub const MODEL_DIR: &str = "models/TTS/zerotts";

pub const MODEL_REVISION: &str =
    "c2bfbd67dc648cac455077333f7cf5c18a2e3bb4";

pub const CORE_ASSETS: &[Asset] = &[
    Asset {
        path: "config.json",
        url: "https://huggingface.co/zeroweight-ai/ZeroTTS/resolve/c2bfbd67dc648cac455077333f7cf5c18a2e3bb4/config.json",
    },
    Asset {
        path: "tokenizer.json",
        url: "https://huggingface.co/zeroweight-ai/ZeroTTS/resolve/c2bfbd67dc648cac455077333f7cf5c18a2e3bb4/tokenizer.json",
    },
    Asset {
        path: "null_voice_emb.npy",
        url: "...",
    },
    Asset {
        path: "silence_frame.npy",
        url: "...",
    },
    Asset {
        path: "onnx/text_encoder.onnx",
        url: "...",
    },
    Asset {
        path: "onnx/prefix_step.onnx",
        url: "...",
    },
    Asset {
        path: "onnx/local_frame_decode.onnx",
        url: "...",
    },
    Asset {
        path: "onnx/codec/moss_audio_tokenizer_decode_full.onnx",
        url: "...",
    },
    Asset {
        path: "onnx/codec/moss_audio_tokenizer_decode_step.onnx",
        url: "...",
    },
    Asset {
        path: "onnx/codec/moss_audio_tokenizer_decode_shared.data",
        url: "...",
    },
    Asset {
        path: "onnx/codec/codec_browser_onnx_meta.json",
        url: "...",
    },
    Asset {
        path: "onnx/codec/LICENSE-Apache-2.0.txt",
        url: "...",
    },
];
```

Phải lấy chính xác URL hiện đang dùng trong `models/manifest.toml` trước khi xóa manifest.

---

## 7. ZeroTTS phải tải toàn bộ voices

Không lazy-download từng voice.

Khi ZeroTTS assets được chuẩn bị:

```text
ensure_assets()
    ├── ensure tất cả CORE_ASSETS
    └── ensure tất cả VOICES
```

Danh sách hiện tại phải bao gồm tối thiểu:

```text
maichi
baotrang
giahuy
hamy
huuduc
kimoanh
quangminh
tiendat
```

Ví dụ:

```rust
pub const VOICES: &[VoiceAsset] = &[
    VoiceAsset {
        id: "maichi",
        name: "Mai Chi",
        path: "voices/maichi/voice.npz",
        url: "https://huggingface.co/zeroweight-ai/ZeroTTS/resolve/.../voices/maichi/voice.npz",
    },
    VoiceAsset {
        id: "baotrang",
        name: "Bao Trang",
        path: "voices/baotrang/voice.npz",
        url: "...",
    },
    // ...
];
```

Khi upstream có thêm voice và project muốn support voice đó, chỉ cần update `VOICES`.

Không lưu URL voice trong database.

---

## 8. Dùng chung danh sách voice cho descriptor

Hiện ZeroTTS descriptor có danh sách `VOICES` riêng.

Không nên duy trì hai danh sách:

```text
assets.rs    -> voice download
descriptor.rs -> voice API/UI
```

vì dễ lệch nhau.

Refactor để metadata voice có một nguồn duy nhất.

Có thể đặt:

```rust
pub struct ZeroTtsVoice {
    pub id: &'static str,
    pub name: &'static str,
    pub path: &'static str,
    pub url: &'static str,
}
```

Sau đó:

```text
assets.rs
   ├── ensure all voices
   └── expose voice metadata
            ↓
descriptor.rs
```

Nếu lifetime/static constraints của `VoiceOption` khiến việc map runtime khó, có thể tách một module:

```text
voice_catalog.rs
```

nhưng dữ liệu vẫn chỉ được khai báo một lần.

Acceptance rule:

```text
Voice được expose qua descriptor
=
Voice được ensure/download
```

Không được có voice chỉ xuất hiện ở một bên.

---

## 9. Quy tắc xác định asset đã sẵn sàng

Không tính SHA.

Không đọc toàn bộ file.

Chỉ kiểm tra:

```rust
pub fn is_ready(path: &Path) -> bool {
    std::fs::metadata(path)
        .map(|metadata| metadata.is_file() && metadata.len() > 0)
        .unwrap_or(false)
}
```

Quy tắc:

```text
file tồn tại + regular file + size > 0
    => READY

không tồn tại / size = 0
    => DOWNLOAD
```

Không kiểm tra `mtime`.

Không kiểm tra checksum.

Không parse ONNX trong asset preparation.

Việc model có load được hay không sẽ do runtime initialization xác nhận.

---

## 10. Download bắt buộc dùng `.part`

Không download trực tiếp vào final path.

Sai:

```text
download
  ↓
models/TTS/zerotts/onnx/text_encoder.onnx
```

Đúng:

```text
models/TTS/zerotts/onnx/text_encoder.onnx.part
                       ↓
                   download
                       ↓
                 validate size > 0
                       ↓
                 atomic rename
                       ↓
models/TTS/zerotts/onnx/text_encoder.onnx
```

Pseudo-code:

```rust
fn ensure_asset(asset: &Asset) -> Result<PathBuf, AssetError> {
    let target = Path::new(MODEL_DIR).join(asset.path);

    if is_ready(&target) {
        return Ok(target);
    }

    let _guard = asset_lock(&target);

    if is_ready(&target) {
        return Ok(target);
    }

    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let part = PathBuf::from(format!("{}.part", target.display()));

    let _ = std::fs::remove_file(&part);

    download(asset.url, &part)?;

    let metadata = std::fs::metadata(&part)?;
    if !metadata.is_file() || metadata.len() == 0 {
        let _ = std::fs::remove_file(&part);
        return Err(AssetError::InvalidDownload);
    }

    std::fs::rename(&part, &target)?;

    Ok(target)
}
```

Nếu download lỗi:

```text
remove .part
return error
```

Không để file final tồn tại khi download chưa hoàn tất.

---

## 11. Giữ per-asset lock

Mặc dù loại bỏ Model Preparation framework, vẫn phải chống duplicate download.

Tình huống:

```text
WS A → cần ZeroTTS
WS B → cần ZeroTTS
```

Hai request không được tải cùng một file đồng thời.

Flow:

```text
check READY
   │
   └── false
        ↓
     acquire lock
        ↓
     check READY lần 2
        │
        ├── true  -> provider khác vừa tải xong
        └── false -> download
```

Có thể reuse striped lock implementation hiện tại nếu nó đơn giản và không phụ thuộc model manifest.

Không cần distributed lock.

Server hiện là một process homelab.

---

## 12. `ensure_assets()` trả về paths runtime cần

Không để runtime tự đoán path rải rác.

Ví dụ ZeroTTS:

```rust
pub struct ZeroTtsAssets {
    pub config: PathBuf,
    pub tokenizer: PathBuf,
    pub null_voice: PathBuf,
    pub silence_frame: PathBuf,
    pub text_encoder: PathBuf,
    pub prefix_step: PathBuf,
    pub local_frame_decode: PathBuf,
    pub codec_decode_full: PathBuf,
    pub codec_decode_step: PathBuf,
    pub codec_shared_data: PathBuf,
    pub codec_metadata: PathBuf,
    pub codec_license: PathBuf,
    pub voices: BTreeMap<String, PathBuf>,
}
```

`ensure_assets()`:

```rust
pub fn ensure_assets() -> Result<ZeroTtsAssets, AssetError>
```

Sau khi hàm trả về thành công:

- toàn bộ core files tồn tại;
- toàn bộ supported voices tồn tại;
- runtime nhận paths đã resolve.

Runtime ZeroTTS không được download file.

Runtime ZeroTTS không được checksum file.

---

## 13. Không download model ở generic server startup

Xóa flow:

```text
startup_with_lifecycle()
    ↓
prepare_startup()
    ↓
preparing startup model artifacts
```

Generic startup mới:

```text
load config
   ↓
connect database
   ↓
run migrations
   ↓
build provider registry
   ↓
create ProviderRuntimeManager
   ↓
bind HTTP / WS
```

Không scan:

```text
models/TTS
models/ASR
models/VAD
```

Không verify provider model chưa được dùng.

---

## 14. Khi nào tải model

Model được ensure khi provider thực sự được runtime manager materialize.

Flow:

```text
session/template
      ↓
resolve provider
      ↓
ProviderRuntimeManager::get_or_load(...)
      ↓
prepare_artifacts / ensure_assets
      ↓
missing files?
   ├── yes -> download
   └── no  -> continue immediately
      ↓
build runtime
      ↓
Ready
```

Provider có `preload=true`:

```text
startup
   ↓
runtime manager preload
   ↓
ensure_assets()
   ↓
load provider runtime
```

Provider không preload:

```text
không tải model cho đến khi provider được dùng
```

---

## 15. `FactoryMaterializer::prepare_artifacts`

Hiện tại:

```text
crates/voice-agent-server/src/services/provider_runtime/factory.rs
```

đang gọi:

```rust
crate::models::prepare_immutable(...)
```

Phải loại bỏ dependency này.

Có hai phương án.

### Khuyến nghị: adapter đăng ký AssetProvider

Mở rộng registration:

```rust
pub trait ProviderAssetManager: Send + Sync {
    fn ensure_assets(&self) -> Result<(), ProviderAssetError>;
}
```

Registration:

```rust
pub struct ProviderAdapterRegistration {
    pub descriptor: &'static ProviderDescriptor,
    pub bootstrap_inspector: Option<&'static dyn BootstrapCapabilityInspector>,
    pub assets: Option<&'static dyn ProviderAssetManager>,
}
```

ZeroTTS:

```rust
static ASSET_MANAGER: ZeroTtsAssetManager = ZeroTtsAssetManager;

pub static REGISTRATION: ProviderAdapterRegistration = ProviderAdapterRegistration {
    descriptor: &DESCRIPTOR,
    bootstrap_inspector: Some(&INSPECTOR),
    assets: Some(&ASSET_MANAGER),
};
```

Factory:

```rust
fn prepare_artifacts(
    &self,
    snapshot: &DesiredProvider,
) -> Result<(), RuntimeError> {
    let registry = crate::providers::compiled_provider_adapter_registry();

    let registration = registry
        .get(&snapshot.adapter)
        .ok_or(RuntimeError::Configuration)?;

    if let Some(assets) = registration.assets {
        assets
            .ensure_assets()
            .map_err(|_| RuntimeError::ArtifactsNotReady)?;
    }

    Ok(())
}
```

Mục tiêu là Factory không chứa:

```rust
match snapshot.adapter.as_str()
```

cho từng provider.

Provider ownership phải được giữ rõ ràng.

---

## 16. `build()` không gọi prepare/checksum lần hai

Hiện tại `FactoryMaterializer::build()` còn gọi lại:

```rust
crate::models::prepare_immutable(...)
```

Phải bỏ.

Không được có flow:

```text
prepare_artifacts()
    ↓
ensure/download
    ↓
build()
    ↓
ensure lại toàn bộ
```

`build()` chỉ:

```text
resolve provider-specific asset paths
   ↓
construct provider runtime
```

Có thể cho `assets.rs` cung cấp:

```rust
pub fn resolve_assets() -> Result<ZeroTtsAssets, AssetError>
```

Hàm này chỉ kiểm tra required paths và trả struct paths.

Nếu cần defensive check, chỉ dùng `metadata`/`size > 0`.

Không SHA.

---

## 17. Bỏ `models/manifest.toml`

Sau khi tất cả local providers đã chuyển sang provider-owned assets:

Xóa:

```text
models/manifest.toml
```

Không giữ compatibility path.

Không đọc manifest ở startup.

Không giữ manifest chỉ để lấy URL.

URL phải nằm ở provider `assets.rs`.

---

## 18. Xóa Model Preparation framework cũ

Review:

```text
crates/voice-agent-server/src/models.rs
crates/voice-agent-server/src/models/startup.rs
crates/voice-agent-server/src/models/acquisition.rs
```

Các phần sau phải được xóa hoặc thay thế bởi asset downloader tối giản:

```text
Manifest
Model
Artifact.source_sha256
Artifact.sha256

ModelPreparation
ModelPreparationConfig

prepare()
prepare_startup()

prepare_immutable()
prepare_immutable_inner()

verify_installed()

model_fingerprint()
fingerprint_model()

verify_path()
verifies()
hash_file()

MissingAcknowledgement
LicenseDenied
HashMismatch
```

Không bắt buộc phải xóa nguyên module `models` nếu downloader chung vẫn phù hợp.

Có thể đổi thành:

```text
src/assets/
```

hoặc:

```text
src/providers/assets.rs
```

Nhưng không để abstraction/model naming cũ tiếp tục tạo sự phức tạp không cần thiết.

---

## 19. Loại bỏ config deployment model cũ

Tìm toàn bộ usage của:

```text
deployment.model_manifest
deployment.models.root
deployment.models.offline
deployment.models.sources
deployment.model_acknowledgements
```

Loại bỏ khỏi:

- config struct;
- config defaults;
- validation;
- example config;
- tests;
- qualification tools;
- docs;
- startup.

Nếu một field chỉ phục vụ Model Preparation cũ thì xóa hẳn.

Không để deprecated config tồn tại nhưng không dùng.

Config provider không được thêm field thay thế cho các field trên.

---

## 20. Provider config giữ tối giản

Ví dụ ZeroTTS:

```json
{
  "voice": "maichi",
  "language": "vi-VN",
  "delivery_mode": "stream",
  "preload": false
}
```

Không expose:

```text
model
model_path
models_root
revision
download_url
sha256
offline
num_threads
onnx_library
```

`num_threads` tiếp tục lấy từ runtime/server internal settings hiện tại.

Không cho Admin API chỉnh asset URL.

---

## 21. ResourceKey không còn dùng model SHA/fingerprint

Hiện `FactoryMaterializer::resource_key_with_fingerprint()` có dependency vào:

```rust
model_fingerprint(...)
```

Phải bỏ.

Resource identity của local runtime nên dựa trên stable provider identity/version nội bộ.

Ví dụ:

```text
adapter
+ MODEL_REVISION
+ ONNX execution settings
+ threads
+ physical runtime parameters
```

Không đọc model file để tạo key.

Ví dụ ZeroTTS physical key:

```text
zerotts_onnx
+ c2bfbd67...
+ onnx runtime identity
+ threads
```

Không đưa selected `voice` vào physical ZeroTTS ResourceKey nếu kiến trúc hiện tại đã chuyển sang:

```text
one physical ZeroTTS engine
→ many logical voice bindings
```

Voice chỉ thuộc logical provider binding.

---

## 22. Qualification manifest fingerprint

Hiện `FactoryMaterializer` có:

```text
qualified_manifest_fingerprint
verify_qualified_manifest()
manifest_fingerprint()
measured_manifest_sha256
```

Các khái niệm này phụ thuộc `models/manifest.toml`.

Sau refactor:

- bỏ model manifest fingerprint;
- không hash manifest;
- không hash model artifacts.

Nếu qualification vẫn cần nhận biết build/runtime version, dùng thông tin khác, ví dụ:

```text
adapter
asset revision constant
runtime execution config
ONNX runtime version/path identity
```

Không tái tạo một manifest SHA system khác dưới tên mới.

---

## 23. Provider-specific transform

Một số provider không thể chỉ download file trực tiếp.

Ví dụ hiện tại có:

### Zipformer

```text
bpe.model
   ↓
sentencepiece_tokens_v1
   ↓
tokens.txt
```

### Kokoro

```text
voicepack.pt
   ↓
kokoro_voicepack_v1
   ↓
voicepack.bin
```

Các transform này phải chuyển về `assets.rs` của provider.

Ví dụ:

```rust
if !is_ready(&tokens_txt) {
    ensure_download(&bpe_source)?;
    convert_sentencepiece(&bpe_source, &tokens_part)?;
    rename(tokens_part, tokens_txt)?;
}
```

Không tạo generic manifest field:

```text
transform = "..."
```

Provider nào cần transform thì provider đó tự implement.

Final transformed file mới là file dùng để kiểm tra `exists + size > 0`.

---

## 24. Error handling

Tạo error đơn giản:

```rust
pub enum AssetError {
    Io(std::io::Error),
    Download(String),
    InvalidDownload(PathBuf),
    Transform(String),
    Missing(PathBuf),
}
```

Không cần error variants:

```text
HashMismatch
MissingAcknowledgement
LicenseDenied
UnknownModel
ManifestParse
Offline
```

Log phải cho biết:

```text
provider
asset
url
destination
operation
```

Không log token hoặc secret.

Ví dụ:

```text
INFO provider=zerotts asset=text_encoder "downloading provider asset"
INFO provider=zerotts asset=text_encoder "provider asset ready"
INFO provider=zerotts asset=text_encoder "reusing provider asset"
```

---

## 25. Download retry

Không cần framework retry phức tạp.

Một lần `ensure_assets()` có thể fail nếu network lỗi.

Runtime manager trả provider unavailable.

Lần request/preload tiếp theo có thể thử lại.

Quan trọng:

```text
failed download
→ không tạo final file
→ remove .part
```

Do đó retry sau này sạch.

---

## 26. Không tự động xóa model cũ

Refactor này không cần model garbage collector.

Nếu provider revision thay đổi và path vẫn giữ nguyên:

```text
models/TTS/zerotts/...
```

thì file cũ sẽ được coi là tồn tại.

Vì vậy khi thay model upstream mà vẫn cùng filename, developer phải chủ động một trong hai:

1. đổi relative path/file name; hoặc
2. xóa file model cũ trong deployment khi update.

Khuyến nghị khi có breaking model update:

```text
models/TTS/zerotts/v2/...
```

hoặc đổi tên file/folder provider.

Không cần tự động checksum để phát hiện thay đổi.

Đây là trade-off được chấp nhận trong thiết kế mới.

---

## 27. Tests bắt buộc

### 27.1 Existing asset is reused

Setup:

```text
target file tồn tại
size > 0
```

Expectation:

```text
không gọi HTTP downloader
```

---

### 27.2 Missing asset is downloaded

Setup:

```text
target không tồn tại
```

Expectation:

```text
download .part
rename final
final size > 0
```

---

### 27.3 Zero-byte file được tải lại

Setup:

```text
target tồn tại
size = 0
```

Expectation:

```text
không reuse
download lại
```

---

### 27.4 Interrupted download không tạo final asset

Downloader trả lỗi giữa chừng.

Expectation:

```text
final path không tồn tại
.part được cleanup
```

---

### 27.5 Concurrent ensure chỉ download một lần

Hai threads/tasks gọi cùng provider.

Expectation:

```text
mỗi missing asset chỉ download một lần
```

---

### 27.6 ZeroTTS tải toàn bộ voices

Setup model folder empty.

Call:

```rust
zerotts::assets::ensure_assets()
```

Expectation:

```text
maichi exists
baotrang exists
giahuy exists
hamy exists
huuduc exists
kimoanh exists
quangminh exists
tiendat exists
```

Không chỉ selected voice.

---

### 27.7 Descriptor và assets voice catalog đồng bộ

Assert:

```text
descriptor voice IDs == asset voice IDs
```

---

### 27.8 Generic startup không chạm model files

Start server với model folder empty và không preload local provider.

Expectation:

```text
server bind thành công
models không bị tải
không có log "preparing startup model artifacts"
```

---

### 27.9 Runtime load downloads missing assets

Provider được materialize.

Expectation:

```text
ensure assets
then build runtime
```

---

### 27.10 Second runtime load không download lại

Sau lần đầu đã có files:

```text
ensure assets
→ metadata only
→ build
```

Downloader call count không tăng.

---

## 28. Xóa/update tests cũ

Các tests phụ thuộc vào hành vi sau phải được xóa hoặc rewrite:

```text
SHA mismatch
startup repairs corrupt immutable copy
hot preparation refuses immutable replacement
model manifest acknowledgement
offline model preparation
manifest fingerprint
.installed tree
checksum source/output
```

Không giữ tests để bảo vệ behavior đã bị loại bỏ.

---

## 29. Documentation cleanup

Search toàn repository:

```bash
rg "model_manifest|manifest.toml|source_sha256|sha256|prepare_immutable|prepare_startup|model_acknowledgements|models.offline|models.root|models.sources|measured_manifest_sha256"
```

Update docs liên quan.

Đặc biệt kiểm tra:

```text
docs/04-configuration.md
docs/06-implementation-plan.md
docs/flow.md
docs/flows/*
docs/testing/*
docs/adr/0044-pinned-startup-model-preparation.md
docs/provider-runtime-manager-qualification.md
```

ADR 0044 không còn đúng.

Không chỉnh ADR Accepted theo kiểu làm như thiết kế cũ chưa từng tồn tại.

Nên:

- đánh dấu ADR 0044 là `Superseded`; và
- thêm ADR mới mô tả Provider-Owned Model Assets.

---

## 30. ADR mới nên ghi rõ trade-off

ADR mới cần ghi:

### Decision

```text
Provider owns its model assets.
```

### Integrity model

```text
exists + regular file + size > 0
```

Không cryptographic verification.

### Download safety

```text
.part + atomic rename + per-asset lock
```

### Versioning

```text
upstream URLs are pinned in provider source code
```

### Configuration

```text
users cannot configure model location or download URL
```

### Known trade-off

Nếu file model trên disk bị sửa/corrupt nhưng vẫn có size > 0:

```text
asset layer sẽ reuse file
```

Runtime initialization có thể fail sau đó.

Đây là behavior được chấp nhận để giữ hệ thống đơn giản và startup nhanh.

---

## 31. Thứ tự triển khai khuyến nghị

### Bước 1 — Tạo asset foundation tối giản

Tạo:

```text
ProviderAssetManager
Asset
download_atomic()
is_ready()
asset lock
```

Chưa xóa Model Preparation cũ.

---

### Bước 2 — Migrate ZeroTTS trước

Tạo:

```text
providers/tts/zerotts/assets.rs
```

Chuyển:

- core URLs;
- all voice URLs;
- paths.

Update ZeroTTS runtime để dùng resolved paths từ assets.

Thêm tests.

---

### Bước 3 — Migrate các local providers còn lại

Theo thứ tự:

```text
Silero
Gipformer
Zipformer
Kokoro VI
Speaker Recognition providers nếu có
```

Provider remote như:

```text
openai
chillaudio_ws
```

không có asset manager.

---

### Bước 4 — Chuyển Runtime Manager

Update:

```text
FactoryMaterializer::prepare_artifacts()
FactoryMaterializer::build()
ResourceKey
```

Loại bỏ `prepare_immutable()`.

---

### Bước 5 — Xóa startup model preparation

Xóa:

```text
prepare_startup()
spawn_blocking Model Preparation
"preparing startup model artifacts"
```

Verify server có thể start khi model folder chưa tồn tại.

---

### Bước 6 — Xóa manifest/framework cũ

Xóa:

```text
models/manifest.toml
model SHA/fingerprint logic
immutable tree
acknowledgements
offline/source config
```

---

### Bước 7 — Cleanup config/docs/tests

Không để dead config hoặc compatibility behavior còn sót.

---

## 32. Không được làm

Agent không được thay Model Preparation cũ bằng một framework mới có độ phức tạp tương đương.

Không thêm:

```text
asset database
asset manifest JSON
asset state machine
checksum cache
mtime cache
background integrity scanner
download scheduler
model registry service
```

trừ khi có yêu cầu riêng sau này.

Không download mọi model của mọi provider khi server startup.

Không đưa model path/URL ra Admin API.

Không dùng selected ZeroTTS voice để quyết định voice nào được download: ZeroTTS phải đảm bảo **all supported voices**.

---

## 33. Tiêu chí nghiệm thu cuối cùng

Implementation được xem là hoàn thành khi đáp ứng toàn bộ:

- [ ] Không còn `models/manifest.toml`.
- [ ] Không còn SHA-256 verification cho model artifacts.
- [ ] Không còn `.installed/<fingerprint>`.
- [ ] Không còn `prepare_startup()` model scan.
- [ ] Generic server startup không tải model.
- [ ] Config không có model root.
- [ ] Config không có offline model mode.
- [ ] Config không có model source URL.
- [ ] Mỗi local provider có asset ownership rõ ràng.
- [ ] Model URL nằm trong provider source (`assets.rs`).
- [ ] File có sẵn và size > 0 được reuse ngay.
- [ ] Missing file tự download.
- [ ] Download dùng `.part` + atomic rename.
- [ ] Concurrent ensure không duplicate download.
- [ ] ZeroTTS tải toàn bộ supported voices.
- [ ] ZeroTTS descriptor và voice asset catalog không lệch nhau.
- [ ] Runtime không download/checksum model trực tiếp.
- [ ] Provider Runtime Manager materialize provider sau khi assets ready.
- [ ] Provider API/config không expose model implementation details.
- [ ] Tests cũ phụ thuộc manifest/SHA/offline đã được cleanup.
- [ ] Documentation phản ánh đúng architecture mới.

---

## 34. Flow cuối cùng mong muốn

### Server startup

```text
Config
  ↓
Database
  ↓
Migrations
  ↓
Provider Registry
  ↓
Provider Runtime Manager
  ↓
HTTP / WS Bind
```

### Provider materialization

```text
Template / Session cần provider
             ↓
    RuntimeManager::get_or_load
             ↓
      provider.ensure_assets
             ↓
     ┌──── existing? ────┐
     │                   │
    yes                 no
     │                   │
     │             download .part
     │                   │
     │             atomic rename
     │                   │
     └─────────┬─────────┘
               ↓
          build runtime
               ↓
             Ready
```

### ZeroTTS

```text
ensure_assets
   ├── ensure all core model files
   └── ensure ALL supported voices
               ↓
         resolve asset paths
               ↓
         build physical engine
               ↓
       logical voice binding
```

Đây là behavior đích cần giữ xuyên suốt quá trình refactor.
