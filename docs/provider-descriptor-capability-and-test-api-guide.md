# Provider Descriptor, Capability Catalog và Provider Test API

## Mục tiêu

Tài liệu này hướng dẫn bổ sung vào Rust Voice Agent Server hai capability phục vụ Web Admin UI và vận hành provider sau khi hệ thống đã có SQLite-backed Agent / Template / Provider / MCP / Device / History:

1. **Provider Descriptor / Capability Catalog**
   - Web UI biết mỗi adapter hỗ trợ loại provider nào.
   - ASR/TTS/LLM expose model, voice, language và capability cần thiết.
   - UI có thể dựng form cấu hình provider mà không hardcode từng adapter.
   - DB `providers` chỉ lưu desired configuration của một provider instance, không trở thành capability catalog.

2. **Provider Test API**
   - Admin có thể test ASR, LLM và TTS để biết runtime hiện tại trả kết quả gì.
   - Test không thay đổi Voice Session, Template, Provider binding hoặc `RuntimeCatalog`.
   - Test không hot-load provider chưa có runtime.
   - Test không ghi `history_messages`.
   - Credential vẫn chỉ đi qua `secret_ref` / `SecretResolver`; API test không phải secret-test endpoint.

Baseline kiến trúc được giả định:

- SQLite provider rows đã có `key`, `type`, `adapter`, typed `config_json`, `secret_ref`, `enabled`, `revision`.
- `ProviderCatalog` / `RuntimeCatalog` là application-owned.
- Provider runtime đã snapshot/load theo startup plan required/optional.
- `runtime_status`, `runtime_matches_desired`, `requires_restart` đã được định nghĩa.
- Provider config đã có `ProviderConfigValidator` dùng chung cho Admin API và startup.
- Admin API nằm dưới `/api/admin`, có Bearer token riêng, request-id server-side, optimistic concurrency và body bounds.

---

# 1. Nguyên tắc kiến trúc

Không thêm các cột kiểu:

```text
providers.voice
providers.model
providers.language
providers.voices_json
providers.models_json
```

vào bảng `providers` để mô tả capability của adapter.

Lý do:

- Một adapter có thể hỗ trợ nhiều model.
- Một model có thể hỗ trợ nhiều voice/language.
- Capability có thể phụ thuộc runtime/model đã load.
- Danh sách model/voice có thể đến từ adapter code hoặc discovery động.
- Nếu duplicate capability vào DB, UI metadata rất dễ lệch khỏi implementation thật.

Tách rõ:

```text
Provider Adapter Descriptor
    = adapter hỗ trợ gì / cấu hình bằng field nào

Provider Instance
    = một cấu hình cụ thể được lưu trong DB

Provider Runtime
    = runtime hiện đã load trong process
```

Ví dụ:

```text
Adapter: zerotts_onnx
    supports voices: maichi, male_01, ...
    supports language: vi-VN
    supports streaming: true

Provider instance: zerotts_maichi
    adapter = zerotts_onnx
    config = {
        model: zerotts,
        voice: maichi,
        language: vi-VN
    }
```

---

# 2. Module layout đề xuất

```text
crates/voice-agent-server/src/
├── providers/
│   ├── registry.rs
│   ├── descriptor.rs
│   ├── capabilities.rs
│   ├── inspector.rs
│   ├── diagnostics.rs
│   │
│   ├── asr/
│   │   ├── gipformer/
│   │   │   ├── config.rs
│   │   │   ├── descriptor.rs
│   │   │   └── provider.rs
│   │   └── zipformer/
│   │       ├── config.rs
│   │       ├── descriptor.rs
│   │       └── provider.rs
│   │
│   ├── tts/
│   │   ├── zerotts/
│   │   │   ├── config.rs
│   │   │   ├── descriptor.rs
│   │   │   └── provider.rs
│   │   └── chillaudio/
│   │       ├── config.rs
│   │       ├── descriptor.rs
│   │       └── provider.rs
│   │
│   └── llm/
│       └── openai/
│           ├── config.rs
│           ├── descriptor.rs
│           └── provider.rs
│
├── services/
│   └── provider_diagnostic.rs
│
└── admin/
    ├── provider_adapters.rs
    ├── providers.rs
    └── provider_tests.rs
```

Mỗi adapter nên sở hữu cùng lúc:

```text
Typed Config
Descriptor
Capabilities
Validation
Factory / Runtime Builder
Diagnostic implementation
```

Không tạo một capability registry rời hoàn toàn khỏi adapter implementation, vì sẽ drift theo thời gian.

---

# 3. Provider types

Nên có enum chung:

```rust
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderType {
    Vad,
    Asr,
    Llm,
    Tts,
    Vision,
}
```

Trong scope hiện tại, Web UI/test API tập trung:

```text
ASR
LLM
TTS
```

VAD chưa cần test API public trong phase này.

---

# 4. ProviderDescriptor

Descriptor là metadata immutable của adapter implementation.

Đề xuất:

```rust
pub struct ProviderDescriptor {
    pub adapter: &'static str,
    pub provider_type: ProviderType,
    pub display_name: &'static str,
    pub description: &'static str,
    pub config_schema: ProviderConfigSchema,
    pub capabilities: ProviderCapabilities,
    pub discovery: CapabilityDiscoveryMode,
}
```

## 4.1 Capability discovery mode

```rust
pub struct CapabilityDiscoveryMode {
    pub models: DiscoverySource,
    pub voices: DiscoverySource,
    pub languages: DiscoverySource,
}

pub enum DiscoverySource {
    Unsupported,
    Static,
    Bootstrap,
    Runtime,
    BootstrapAndRuntime,
    Remote,
}
```

Ví dụ:

```text
ZeroTTS
models     = Static
voices     = BootstrapAndRuntime
languages  = Static

OpenAI-compatible LLM
models     = Remote hoặc Unsupported trong V1
voices     = Unsupported
languages  = Unsupported
```

---

# 5. UI-neutral Provider Config Schema

Server không trả HTML/UI framework config. Server chỉ trả metadata semantic để Web UI tự render.

```rust
pub struct ProviderConfigSchema {
    pub fields: Vec<ProviderConfigField>,
}

pub struct ProviderConfigField {
    pub key: &'static str,
    pub label: &'static str,
    pub field_type: ConfigFieldType,
    pub required: bool,
    pub nullable: bool,
    pub enum_source: Option<CapabilitySource>,
    pub enum_values: Option<Vec<&'static str>>,
    pub minimum: Option<i64>,
    pub maximum: Option<i64>,
    pub max_length: Option<usize>,
    pub description: Option<&'static str>,
}
```

```rust
pub enum ConfigFieldType {
    String,
    Integer,
    Boolean,
    Select,
}

pub enum CapabilitySource {
    Models,
    Voices,
    Languages,
}
```

Ví dụ response cho ZeroTTS:

```json
{
  "adapter": "zerotts_onnx",
  "type": "tts",
  "display_name": "ZeroTTS",
  "config_schema": {
    "fields": [
      {
        "key": "model",
        "label": "Model",
        "type": "select",
        "required": true,
        "enum_source": "models"
      },
      {
        "key": "voice",
        "label": "Voice",
        "type": "select",
        "required": true,
        "enum_source": "voices"
      },
      {
        "key": "language",
        "label": "Language",
        "type": "select",
        "required": true,
        "enum_source": "languages"
      },
      {
        "key": "num_threads",
        "label": "Threads",
        "type": "integer",
        "required": true,
        "minimum": 1,
        "maximum": 128
      },
      {
        "key": "preload",
        "label": "Preload",
        "type": "boolean",
        "required": false
      },
      {
        "key": "delivery_mode",
        "label": "Delivery mode",
        "type": "select",
        "required": false,
        "enum_values": ["stream", "file"]
      }
    ]
  }
}
```

Frontend tự quyết định:

```text
select     -> <select>
boolean    -> checkbox/switch
integer    -> number input
string     -> text input
```

Không để backend phụ thuộc React/Vue/Svelte.

Descriptor field keys, required/default semantics và bounds phải khớp typed config mà
`ProviderConfigValidator` cùng runtime factory thực sự chấp nhận. Không được publish một field
descriptor-only rồi hy vọng runtime bỏ qua nó.

Typed config được mở rộng trên cơ sở config hiện có:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ZeroTtsConfig {
    pub model: String,
    pub num_threads: i32,
    pub voice: String,
    pub language: String,
    #[serde(default)]
    pub preload: bool,
    #[serde(default)]
    pub delivery_mode: ZeroTtsDeliveryMode,
}
```

`language` phải được thêm đồng thời vào desired-config validator, runtime config, adapter semantic
validation và descriptor. `voice` là field hiện có và tiếp tục là typed config, không phải metadata
UI riêng.

---

# 6. Capability model cho ASR

```rust
pub struct AsrCapabilities {
    pub models: Vec<ModelOption>,
    pub languages: Vec<LanguageOption>,
    pub streaming: bool,
    pub offline: bool,
    pub input_sample_rates: Vec<u32>,
    pub channels: Vec<u8>,
}
```

```rust
pub struct ModelOption {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
}

pub struct LanguageOption {
    pub id: String,
    pub name: String,
}
```

Ví dụ Gipformer:

```json
{
  "adapter": "gipformer_sherpa_offline",
  "type": "asr",
  "capabilities": {
    "models": [
      {
        "id": "gipformer1.5-68m-rnnt",
        "name": "Gipformer 1.5 68M RNNT"
      }
    ],
    "languages": [
      {
        "id": "vi-VN",
        "name": "Vietnamese"
      }
    ],
    "streaming": false,
    "offline": true,
    "input_sample_rates": [16000],
    "channels": [1]
  }
}
```

Typed provider instance config có thể là:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GipformerConfig {
    pub model: String,
    pub language: String,
    pub num_threads: i32,
    pub decoding_method: String,
    pub max_active_paths: i32,
}
```

Adapter validation phải kiểm tra model/language instance chọn có hợp lệ với adapter capability hiện tại.

Việc thêm `language` cho ZeroTTS/Gipformer là migration bắt buộc. Migration phải backfill giá trị
tương thích cho row cũ hoặc cung cấp default deserialize/canonicalization tương đương trước khi bật
validator mới; không được khiến desired provider đang hợp lệ trở thành `provider_config_invalid` sau
nâng cấp.

---

# 7. Capability model cho TTS

```rust
pub struct TtsCapabilities {
    pub models: Vec<ModelOption>,
    pub voices: Vec<VoiceOption>,
    pub languages: Vec<LanguageOption>,
    pub streaming: bool,
    pub provider_output_sample_rates: Vec<u32>,
    pub voice_delivery_sample_rates: Vec<u32>,
}
```

```rust
pub struct VoiceOption {
    pub id: String,
    pub name: String,
    pub languages: Vec<String>,
    pub model: Option<String>,
}
```

Ví dụ:

```json
{
  "adapter": "zerotts_onnx",
  "type": "tts",
  "capabilities": {
    "models": [
      {
        "id": "zerotts",
        "name": "ZeroTTS"
      }
    ],
    "voices": [
      {
        "id": "maichi",
        "name": "Mai Chi",
        "languages": ["vi-VN"],
        "model": "zerotts"
      }
    ],
    "languages": [
      {
        "id": "vi-VN",
        "name": "Vietnamese"
      }
    ],
    "streaming": true,
    "provider_output_sample_rates": [48000],
    "voice_delivery_sample_rates": [24000]
  }
}
```

Hai sample-rate có semantics khác nhau:

```text
provider_output_sample_rates
    = PCM tại provider seam; Provider Test API ghi WAV trực tiếp từ PCM này

voice_delivery_sample_rates
    = sample rate sau canonical Voice delivery conversion
```

Với ZeroTTS, provider-boundary PCM và diagnostic WAV là mono 48 kHz. 24 kHz chỉ thuộc Voice
delivery sau resampling; descriptor không được quảng cáo 24 kHz như provider output.

Provider instance:

```json
{
  "model": "zerotts",
  "voice": "maichi",
  "language": "vi-VN",
  "num_threads": 2
}
```

---

# 8. Capability model cho LLM

LLM descriptor nên có ít nhất:

```rust
pub struct LlmCapabilities {
    pub models: Vec<ModelOption>,
    pub streaming: bool,
    pub tool_calling: bool,
    pub vision: bool,
}
```

Không bắt buộc mọi remote LLM adapter phải query model list trong V1.

Nếu không support discovery:

```json
{
  "adapter": "openai",
  "type": "llm",
  "capabilities": {
    "models": [],
    "streaming": true,
    "tool_calling": true,
    "vision": false
  },
  "discovery": {
    "models": "unsupported"
  }
}
```

`model` khi đó vẫn là typed string trong instance config.

---

# 9. ProviderAdapterRegistry

Registry phía application:

```rust
pub trait ProviderAdapterDescriptor: Send + Sync {
    fn descriptor(&self) -> &ProviderDescriptor;
}
```

Hoặc registry static:

```rust
pub struct ProviderAdapterRegistry {
    descriptors: HashMap<String, Arc<ProviderDescriptor>>,
}
```

API cần:

```rust
impl ProviderAdapterRegistry {
    pub fn list(&self) -> impl Iterator<Item = &ProviderDescriptor>;
    pub fn get(&self, adapter: &str) -> Option<&ProviderDescriptor>;
}
```

Registry phải cùng source với provider factories/config parser để tránh adapter tồn tại nhưng descriptor không tồn tại hoặc ngược lại.

Startup test nên assert:

```text
mọi registered provider adapter
    -> có typed config parser
    -> có descriptor
    -> descriptor.type đúng provider factory type
```

---

# 10. Admin APIs cho Adapter Descriptor

## 10.1 List adapter

```http
GET /api/admin/provider-adapters
Authorization: Bearer <admin_token>
```

Optional filter:

```text
?type=asr
?type=tts
?type=llm
```

Filter phải typed allowlist, không raw SQL.

Response:

```json
{
  "items": [
    {
      "adapter": "gipformer_sherpa_offline",
      "type": "asr",
      "display_name": "Gipformer Sherpa Offline"
    },
    {
      "adapter": "zerotts_onnx",
      "type": "tts",
      "display_name": "ZeroTTS"
    }
  ]
}
```

## 10.2 Adapter detail

```http
GET /api/admin/provider-adapters/{adapter}
```

Trả:

```text
config schema
static capabilities
discovery source
```

Endpoint này không resolve secret.

---

# 11. Static, bootstrap và loaded-runtime capability phải tách riêng

Có ba nguồn capability.

## Static

Có sẵn từ adapter code:

```text
streaming supported
input/output sample rate
config fields
offline/remote
known language/model set
```

## Bootstrap discovery trước khi tạo provider

UI phải lấy được model/voice/language cần cho create form trước khi provider instance và loaded
runtime tồn tại. Dùng adapter-keyed pre-instance discovery:

```http
POST /api/admin/provider-adapters/{adapter}/capabilities/discover
Content-Type: application/json
Authorization: Bearer <admin_token>
```

Request chứa partial typed selection đã có, ví dụ:

```json
{
  "selection": {
    "model": "zerotts"
  }
}
```

Response trả bounded bootstrap options cho bước tiếp theo, ví dụ voices/languages tương thích với
model đã chọn. Bootstrap discovery:

```text
không cần provider key
không cần loaded provider runtime
không persist DB
không resolve secret
không gọi remote bằng credential
không hot-load runtime/model/voice
chỉ đọc adapter metadata, deployment manifest hoặc artifact metadata an toàn
bounded input, timeout và result count/size
```

Interface:

```rust
#[async_trait]
pub trait ProviderBootstrapInspector: Send + Sync {
    async fn discover(
        &self,
        selection: &BootstrapSelection,
    ) -> Result<DiscoveredCapabilities, ProviderInspectError>;
}
```

`BootstrapSelection` phải được typed/validated theo adapter descriptor; không nhận một JSON bag tùy
ý và không cho phép credential fields.

## Loaded runtime / Remote discovery

Phụ thuộc provider instance/runtime:

```text
loaded voice list
model artifact metadata
remote model list
runtime language set
```

Interface:

```rust
#[async_trait]
pub trait ProviderInspector: Send + Sync {
    async fn inspect(
        &self,
        runtime: &ResolvedProviderRuntime,
    ) -> Result<DiscoveredCapabilities, ProviderInspectError>;
}
```

Loaded-runtime discovery dùng để xác minh runtime sau restart, không được dùng làm con đường duy nhất
để bootstrap create form. Không bắt buộc mọi adapter implement loaded-runtime discovery trong V1.

---

# 12. Không dùng dynamic capability cache làm authority

Nếu sau này cần cache để UI nhanh hơn, có thể thêm bảng:

```text
provider_capability_cache
```

nhưng chỉ là optimization.

Authority vẫn là:

```text
Adapter Descriptor
+
Fresh bootstrap discovery cho create flow
+
Current Loaded Runtime
+
Fresh discovery khi được yêu cầu
```

Không bind Template dựa trên stale cache nếu runtime không tồn tại.

---

# 13. Provider runtime status trong Admin response

Provider GET response nên trả rõ:

```json
{
  "id": 12,
  "key": "zerotts_maichi",
  "type": "tts",
  "adapter": "zerotts_onnx",
  "enabled": true,
  "revision": 5,
  "runtime_status": "loaded",
  "runtime_matches_desired": false,
  "requires_restart": true,
  "has_secret_ref": false
}
```

Semantics:

```text
runtime_status
    = runtime hiện tại trong process

runtime_matches_desired
    = runtime hiện tại có build từ đúng desired DB revision/config không

requires_restart
    = desired state chưa thể có hiệu lực hoàn toàn nếu không restart
```

Provider Test API phải trả lại metadata này để user không hiểu nhầm mình đang test config mới trong DB.

---

# 14. Provider Test API architecture

Không để Axum handler trực tiếp gọi provider implementation.

Tạo:

```rust
pub struct ProviderDiagnosticService {
    registry: Arc<RuntimeCatalog>,
    limiter: Arc<ProviderDiagnosticLimiter>,
}
```

Service chịu trách nhiệm:

```text
lookup provider row
validate provider type
lookup currently loaded runtime
check runtime metadata/revision
acquire diagnostic permit
acquire provider capacity với workload class Diagnostic
apply timeout
execute bounded diagnostic
cancel và xác nhận terminal cleanup khi timeout
sanitize response/error
```

Public identity của provider là immutable `key`. Mọi lookup, route, request state và qualification
artifact phải dùng `key`; numeric database ID chỉ được phép xuất hiện như diagnostic metadata và
không được dùng để gọi API hoặc khôi phục `ScenarioState` qua restart.

Handler chỉ:

```text
parse authenticated request
validate body bounds
call service
map typed error -> HTTP
```

---

# 15. V1 test chỉ test runtime hiện đã load

Không hot-load provider bằng test API.

Nếu:

```text
runtime_status != loaded
```

trả:

```text
409 provider_runtime_not_loaded
```

Nếu runtime hiện có nhưng không khớp desired DB state:

```text
runtime_status          = loaded
runtime_matches_desired = false
requires_restart        = true
```

Test vẫn chạy trên runtime đang load, nhưng response phải nói rõ:

```json
{
  "tested_runtime": "loaded",
  "runtime_matches_desired": false,
  "requires_restart": true
}
```

Không resolve `secret_ref` từ Admin test endpoint.

Không build runtime mới.

Không update `RuntimeCatalog`.

---

# 16. Test LLM

Endpoint:

```http
POST /api/admin/providers/{key}/test/llm
Content-Type: application/json
Authorization: Bearer <admin_token>
```

Body:

```json
{
  "input": "Xin chào. Hãy trả lời ngắn gọn."
}
```

Input bound đề xuất:

```text
1..=8 KiB UTF-8
```

Test LLM V1:

```text
no conversation history
no Device MCP
no External MCP
no tool list
no tool continuation
no TTS
```

Request runtime nên là một diagnostic request độc lập.

Response:

```json
{
  "provider_key": "openai_primary",
  "type": "llm",
  "status": "success",
  "result": {
    "text": "Xin chào! Tôi có thể giúp gì cho bạn?"
  },
  "metrics": {
    "elapsed_ms": 842
  },
  "runtime": {
    "runtime_status": "loaded",
    "runtime_matches_desired": true,
    "requires_restart": false
  }
}
```

Bound output đề xuất:

```text
max diagnostic text result = 32 KiB UTF-8
```

Vượt cap -> `provider_invalid_response`.

Không truncate.

---

# 17. Test TTS

Endpoint:

```http
POST /api/admin/providers/{key}/test/tts
Content-Type: application/json
```

Body:

```json
{
  "text": "Xin chào, đây là thử nghiệm giọng nói.",
  "voice": "maichi",
  "language": "vi-VN"
}
```

`voice` và `language` có thể optional:

```text
absent
    -> dùng config provider instance hiện tại

present
    -> test override tạm thời
```

Override chỉ được phép nếu adapter capability/typed validation chấp nhận.

Test override:

```text
không persist DB
không increment revision
không mutate runtime config
```

Override không dừng ở HTTP DTO. Interface diagnostic phải mang typed request xuyên suốt runtime và
adapter seam:

```rust
pub struct TtsDiagnosticRequest {
    pub text: String,
    pub voice: Option<String>,
    pub language: Option<String>,
}

pub trait TtsDiagnostic {
    fn synthesize_diagnostic(
        &self,
        request: &TtsDiagnosticRequest,
        cancellation: &CancellationToken,
        on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
    ) -> Result<TtsDiagnosticTerminal, TtsError>;
}
```

Runtime chỉ chấp nhận override khi tài nguyên voice/language tương ứng đã được materialize trong
loaded runtime. Test API không được hot-load model/voice, mutate runtime hoặc âm thầm bỏ qua field.
Adapter không hỗ trợ override phải trả typed `invalid_test_input` hoặc `provider_unavailable` theo
nguyên nhân, thay vì synthesize bằng voice khác.

Giá trị trùng với voice/language của loaded provider instance luôn là request hợp lệ. Giá trị khác
chỉ hợp lệ khi adapter đã materialize sẵn biến thể đó và descriptor/runtime capability công bố hỗ trợ
diagnostic override.

Text bound đề xuất:

```text
1..=4096 UTF-8 bytes
```

Response thành công nên trả audio binary trực tiếp:

```http
HTTP/1.1 200 OK
Content-Type: audio/wav
X-Provider-Test-Elapsed-Ms: 640
X-Provider-Runtime-Matches-Desired: true
```

Web UI:

```text
fetch
 -> Blob
 -> URL.createObjectURL
 -> <audio controls>
```

Không encode WAV base64 vào JSON.

WAV là provider-boundary mono PCM với sample rate do chunk provider báo. ZeroTTS V1 phải trả WAV
48 kHz và ghi header 48 kHz. Diagnostic này không chạy qua `CanonicalDownlinkPipeline`, không
resample 48 -> 24 kHz và không encode/decode Opus.

Output audio hard cap đề xuất:

```text
16 MiB
```

Nếu runtime tạo payload vượt cap -> fail diagnostic, không stream vô hạn.

---

# 18. Test ASR

ASR cần media body, nên đây là exception explicit khỏi rule Admin JSON body-only.

Endpoint:

```http
POST /api/admin/providers/{key}/test/asr
Content-Type: audio/wav
Authorization: Bearer <admin_token>
```

V1 chỉ nhận WAV PCM.

Không hỗ trợ:

```text
MP3
AAC
FLAC
OGG
arbitrary multipart media
compressed HTTP request body
```

Hard bounds:

```text
max raw upload     = 5 MiB
max audio duration = 30 seconds
channels           = 1
sample rate        = phải thuộc capability adapter
```

Parser phải kiểm tra header WAV trước khi cấp phát/convert toàn bộ nếu có thể.

Response:

```json
{
  "provider_key": "gipformer_primary",
  "type": "asr",
  "status": "success",
  "result": {
    "text": "xin chào tôi đang thử nhận diện giọng nói",
    "language": "vi-VN"
  },
  "metrics": {
    "audio_duration_ms": 4200,
    "elapsed_ms": 310,
    "rtf": 0.074
  },
  "runtime": {
    "runtime_status": "loaded",
    "runtime_matches_desired": true,
    "requires_restart": false
  }
}
```

RTF:

```text
rtf = inference_elapsed_seconds / audio_duration_seconds
```

Chỉ trả RTF khi audio duration > 0.

---

# 19. Admin body limits và media exception

Existing Admin JSON contract:

```text
JSON body <= 256 KiB
no compressed request body
```

Giữ nguyên cho:

```text
LLM test
TTS test
Provider CRUD
Descriptor/filter APIs
```

ASR test là media-specific route:

```text
Content-Type = audio/wav
raw upload <= 5 MiB
Content-Encoding absent/identity only
```

Không nới generic Admin JSON limit để phục vụ ASR.

---

# 20. Dedicated diagnostic concurrency limiter

Không để Web UI test provider chiếm hết Voice capacity.

Config đề xuất:

```toml
[api.provider_tests]
max_concurrency = 2
timeout_ms = 30000
```

Validation đề xuất:

```text
max_concurrency 1..=8
timeout_ms      1000..=120000
```

Có application-owned:

```rust
pub struct ProviderDiagnosticLimiter {
    semaphore: Semaphore,
}
```

Nếu không lấy được permit theo policy immediate/non-blocking:

```text
429 provider_test_busy
```

Không queue vô hạn.

Diagnostic permit không thay thế provider worker capacity; nó chỉ là lớp cap ngoài để admin test
không tạo burst lớn.

Provider runtime phải có workload-aware admission tại chính runtime seam:

```rust
pub enum ProviderWorkloadClass {
    Voice,
    Diagnostic,
}

pub trait ProviderRuntimeAdmission {
    fn try_admit(
        &self,
        workload: ProviderWorkloadClass,
    ) -> Result<ProviderCapacityPermit, ProviderAdmissionError>;
}
```

Invariants:

```text
Voice có reserved capacity.
Diagnostic không được lấy slot cuối cùng dành cho Voice.
Admission và cấp worker permit là một atomic runtime operation.
HTTP handler không check active-count rồi start sau vì có TOCTOU.
Diagnostic không queue vô hạn và không preempt Voice operation đang chạy.
```

Mỗi shared runtime phải cấu hình hoặc cố định `voice_reserved_capacity >= 1` và không lớn hơn total
capacity. Effective diagnostic capacity là phần capacity còn lại sau reservation; reservation áp dụng
trên cùng permit/worker pool thật, không phải một counter song song.

Với pool chỉ có một worker và reserve một slot cho Voice, shared-runtime diagnostic không có capacity
hợp lệ và phải trả `429 provider_test_busy`. Muốn diagnostic chạy, deployment phải cấp thêm capacity;
không được giảm reservation ngầm hoặc để diagnostic làm Voice nhận capacity error.

Mandatory RC-DB5 qualification phải cấu hình total capacity tối thiểu bằng
`voice_reserved_capacity + 1` cho provider type được test. Thiếu capacity là lỗi qualification setup,
không phải lý do để tắt Voice reservation trong binary production được qualification.

---

# 21. Diagnostic timeout

Mỗi test có bounded timeout.

```text
ASR diagnostic <= configured test timeout
LLM diagnostic <= configured test timeout
TTS diagnostic <= configured test timeout
```

Timeout:

```text
504 hoặc 503 provider_test_timeout
```

Nếu toàn bộ Admin API đang dùng coarse 5xx taxonomy, ưu tiên consistency; nhưng error code domain phải là:

```text
provider_test_timeout
```

Không retry diagnostic request ở server.

Timeout HTTP không đồng nghĩa worker đã dừng hoặc capacity đã được giải phóng. Terminal cleanup là
một phần bắt buộc của contract:

```text
execution timeout
    -> request cancellation on the exact lease/operation
    -> wait bounded cleanup grace
    -> terminal acknowledgement
        -> release exact provider capacity permit once
    -> no terminal acknowledgement
        -> quarantine exact worker/slot
        -> mark effective provider capacity degraded/unavailable
```

`ProviderDiagnosticService` giữ ownership của diagnostic permit cho đến khi nhận terminal
acknowledgement hoặc đã ghi nhận quyết định quarantine. Provider capacity chỉ được trả lại sau
terminal acknowledgement; không được release sớm khi chỉ mới gửi cancellation.

HTTP route có total bound:

```text
route_deadline <= execution_timeout + cleanup_grace
```

Nếu cleanup thất bại, response vẫn giữ primary error `provider_test_timeout` và telemetry ghi một
cleanup outcome riêng, không đổi timeout thành success và không tái sử dụng worker chưa xác nhận dừng.

---

# 22. Error taxonomy

Đề xuất:

```text
provider_not_found
provider_disabled
provider_type_mismatch
provider_runtime_not_loaded
provider_test_busy
provider_test_timeout
provider_unavailable
provider_invalid_response
provider_test_failed
invalid_test_input
```

Không trả:

```text
filesystem path
secret_ref
secret value
remote HTTP body
stacktrace
provider internal Debug string
```

Response:

```json
{
  "error": {
    "code": "provider_runtime_not_loaded",
    "request_id": "..."
  }
}
```

---

# 23. Privacy và persistence

Provider test không phải conversation.

Không ghi vào:

```text
history_messages
DialogueHistory
admin_audit_events dưới dạng input/output body
```

Mutation audit chỉ áp dụng khi có mutation DB.

Provider test read/execute operation có thể telemetry:

```text
provider_key
provider_type
adapter
status
elapsed_ms
result_bytes/audio_bytes
```

Không telemetry:

```text
LLM prompt
LLM response
ASR transcript
TTS input text
TTS audio
secret_ref
secret value
```

---

# 24. Provider descriptor và Q48/Q49

Descriptor/config schema không thay thế `ProviderConfigValidator`.

Admin create/update flow vẫn là:

```text
raw JSON body cap
    ↓
provider config <= 64 KiB
    ↓
parse
    ↓
depth <= 16 / nodes <= 512
    ↓
protected credential-key guard
    ↓
typed config deserialize with deny_unknown_fields
    ↓
adapter validation
    ↓
canonical serialize
    ↓
persist
```

UI metadata chỉ giúp client dựng form tốt hơn.

Server không bao giờ tin rằng request hợp lệ chỉ vì form do server descriptor generate.

---

# 25. Descriptor field bounds

Capability/descriptor output cũng phải bounded.

Đề xuất hard bounds:

```text
adapter key              <= 64 bytes
model id                 <= 128 bytes
model display name       <= 128 UTF-8 bytes
voice id                 <= 128 bytes
voice display name       <= 128 UTF-8 bytes
language id              <= 32 bytes
description              <= 2048 UTF-8 bytes
models per adapter       <= 128
voices per adapter       <= 256
languages per adapter    <= 64
config fields            <= 64
```

Nếu runtime discovery vượt cap:

```text
capability_discovery_invalid
```

Không truncate silently.

---

# 26. Web UI flow

## Create TTS provider

```text
GET /api/admin/provider-adapters?type=tts
        ↓
user chọn zerotts_onnx
        ↓
GET /api/admin/provider-adapters/zerotts_onnx
        ↓
UI render model/thread fields
        ↓
user chọn model
        ↓
POST /api/admin/provider-adapters/zerotts_onnx/capabilities/discover
        ↓
UI render voice/language options tương thích
        ↓
user chọn voice/language
        ↓
POST /api/admin/providers
        ↓
provider desired config persisted
        ↓
response runtime_status / requires_restart
        ↓
restart nếu cần
        ↓
POST /api/admin/providers/{key}/test/tts với text/voice/language
        ↓
play provider-boundary WAV 48 kHz
        ↓
bind provider vào Template
```

## Create ASR provider

```text
GET descriptor
        ↓
select model
        ↓
bootstrap discover language options nếu adapter yêu cầu
        ↓
select language
        ↓
create provider
        ↓
restart if runtime not loaded
        ↓
upload WAV to test/asr
        ↓
show transcript + RTF
```

## Create LLM provider

```text
GET descriptor
        ↓
enter base_url/model + secret_ref
        ↓
create provider
        ↓
restart if required
        ↓
test/llm with small prompt
        ↓
show response + latency
```

---

# 27. Runtime capability matching

Khi adapter config chọn:

```text
model
voice
language
```

adapter validation phải enforce known relationships khi capability static/runtime-known.

Ví dụ:

```text
voice maichi supports vi-VN only
```

thì config:

```json
{
  "voice": "maichi",
  "language": "en-US"
}
```

phải reject.

Nếu capability không thể biết cho đến runtime:

```text
Admin CRUD
    -> typed structural validation

runtime load
    -> runtime semantic validation
```

Required provider semantic failure -> startup fail.

Optional provider -> unavailable/excluded.

---

# 28. Capability discovery APIs

## Pre-instance bootstrap discovery — bắt buộc cho adapter có dynamic create options

```http
POST /api/admin/provider-adapters/{adapter}/capabilities/discover
```

Endpoint này cho phép Web UI chọn model rồi lấy voice/language trước khi `POST /providers`. Nó tuân
theo bootstrap contract tại phần 11 và không phụ thuộc provider instance/runtime đã load.

## Loaded-runtime discovery — optional verification

Nếu cần Web UI xác minh runtime hiện tại có voice/model gì sau restart, có thể thêm:

```http
GET /api/admin/providers/{key}/capabilities
```

Contract V1 nếu implement:

```text
chỉ inspect currently loaded runtime
không resolve secret mới
không hot-load
không mutate DB
bounded timeout
bounded result
```

Nếu runtime chưa load:

```text
409 provider_runtime_not_loaded
```

Response merge:

```text
static descriptor
+
runtime discovered capabilities
```

Không cache làm authority.

---

# 29. Testing strategy

## Unit tests — descriptor

Mỗi adapter:

```text
descriptor adapter key đúng
provider type đúng
config field keys match typed config
capability list không duplicate id
voice references valid model/language
bounds pass
```

## Unit tests — config schema consistency

Ví dụ:

```text
descriptor exposes "voice"
but typed config không có voice
→ test fail
```

Không nhất thiết introspect Rust struct tự động; có thể dùng representative valid payload generated từ descriptor và deserialize typed config.

## Integration tests — APIs

```text
GET provider-adapters requires admin auth
GET descriptor returns bounded metadata
bootstrap discovery works before provider instance exists
bootstrap discovery rejects credential/unknown selection fields
all provider instance routes use immutable key
LLM test does not expose tools
TTS test returns correctly tagged provider-boundary WAV 48 kHz for ZeroTTS
TTS override reaches adapter seam and unsupported override fails typed
ASR test rejects MP3
ASR test rejects >5 MiB
ASR test rejects >30 s
runtime not loaded -> 409
runtime stale -> test allowed + runtime_matches_desired=false
provider test does not write history
provider test does not mutate revision
provider test concurrency cap -> 429
diagnostic cannot consume the Voice-reserved last slot
diagnostic timeout waits for terminal cleanup acknowledgement
missing cleanup acknowledgement quarantines exact worker without releasing its capacity
```

## Privacy tests

Capture logs/test response and assert không có:

```text
secret_ref
secret value
test input text
test output text/raw audio
```

---

# 30. Implementation phases

## PD-0 — Typed provider config migration

- Thêm `language` vào ZeroTTS và Gipformer desired/runtime config.
- Giữ đầy đủ field hiện có: ZeroTTS `model`, `num_threads`, `voice`, `preload`, `delivery_mode`;
  Gipformer `model`, `num_threads`, `decoding_method`, `max_active_paths`.
- Backfill/default row cũ trước khi validator mới trở thành authority.
- Đồng bộ `ProviderConfigValidator`, runtime config, factory và semantic validation.
- Regression tests chứng minh config cũ migrate được và unknown fields vẫn fail closed.

PD-1 không được bắt đầu bằng descriptor giả định schema mới khi PD-0 chưa có typed config contract và
migration tương ứng.

## PD-1 — Descriptor core

- Add `ProviderDescriptor`.
- Add capability structs.
- Add UI-neutral config schema structs.
- Add `ProviderAdapterRegistry`.
- Implement descriptors cho adapter đang có.
- Unit tests descriptor/config consistency.

Gate:

```text
cargo test
```

Không thay Voice semantics.

## PD-2 — Admin descriptor APIs

- `GET /api/admin/provider-adapters`
- `GET /api/admin/provider-adapters/{adapter}`
- typed filter by provider type
- shared Admin auth/request-id/error envelope

Không DB mutation.

## PD-3 — Bootstrap và loaded-runtime capability inspection

- Bắt buộc `ProviderBootstrapInspector` cho adapter có dynamic create options.
- `POST /api/admin/provider-adapters/{adapter}/capabilities/discover` với partial typed selection.
- Optional loaded-runtime `ProviderInspector` để verification sau restart.
- Bound result size/count.
- Optional `GET /providers/{key}/capabilities`.

Không persist, resolve secret hoặc hot-load.

## PD-4 — ProviderDiagnosticService

- Diagnostic limiter.
- Workload-aware runtime admission với Voice reservation.
- Runtime lookup.
- runtime stale metadata.
- Common timeout/error mapping.
- Cancellation, bounded cleanup acknowledgement và exact-worker quarantine.

Không HTTP endpoint trước khi service tests pass.

## PD-5 — LLM test

- JSON input.
- tools disabled.
- no history.
- bounded response.

## PD-6 — TTS test

- JSON input.
- Typed `TtsDiagnosticRequest` đi xuyên HTTP/runtime/adapter seam.
- Temporary voice/language override chỉ dùng tài nguyên đã materialize trong loaded runtime.
- Provider-boundary mono WAV; ZeroTTS là 48 kHz, không qua Voice delivery pipeline.
- output size bound.

## PD-7 — ASR test

- audio/wav endpoint.
- raw upload 5 MiB.
- duration <= 30 s.
- mono/sample-rate validation.
- transcript + RTF response.

## PD-8 — Web UI integration contract

- Document frontend flow.
- Confirm no adapter-specific hardcode required except presentation details.

---

# 31. Definition of Done

Feature hoàn thành khi:

- Mọi ASR/TTS/LLM adapter active có `ProviderDescriptor`.
- ZeroTTS/Gipformer typed config, validator, runtime factory và descriptor cùng chấp nhận `language`.
- Existing desired provider rows migrate/default an toàn trước khi schema mới được enforce.
- ASR descriptor expose model/language/audio capability phù hợp.
- TTS descriptor phân biệt provider output và Voice delivery sample rate.
- Web UI có thể dựng create/edit form từ descriptor metadata.
- Web UI lấy được dynamic voice/language options qua pre-instance bootstrap discovery.
- Provider DB rows vẫn chỉ lưu typed desired config, không duplicate capability catalog.
- Dynamic discovery không trở thành DB authority.
- Mọi Provider Test/instance-capability route dùng immutable provider key.
- Test APIs chỉ dùng runtime đã load.
- Test API không hot-load provider.
- Test API không resolve/test secret trực tiếp.
- Test API không ghi conversation history.
- LLM diagnostic không expose tool calls.
- TTS override đi qua typed diagnostic seam và không hot-load tài nguyên.
- ZeroTTS test trả bounded provider-boundary WAV với header 48 kHz chính xác.
- ASR test chỉ nhận bounded WAV PCM V1.
- Dedicated diagnostic concurrency limiter hoạt động.
- Runtime admission dành capacity cho Voice; diagnostic không lấy slot Voice cuối cùng.
- Timeout chỉ release capacity sau terminal acknowledgement; cleanup failure quarantine exact worker.
- Runtime stale state được phản ánh trong response.
- Errors/logs không leak config/secret/input/output nội dung.
- Existing Voice Session behavior không đổi.

---

# 32. Contract tổng thể

```text
ProviderAdapterRegistry
        │
        ├── Descriptor
        │      ├── config schema
        │      ├── models
        │      ├── voices
        │      ├── languages
        │      └── capabilities
        │
        ├── ProviderBootstrapInspector
        │      └── pre-instance model/voice/language options
        │
        └── Adapter implementation
               ├── typed config
               ├── validation
               └── runtime factory

SQLite providers
        │
        └── desired instance config

RuntimeCatalog
        │
        └── loaded provider runtime
               │
               ├── workload-aware admission
               │      ├── Voice reserved capacity
               │      └── Diagnostic capacity
               │
               └── ProviderDiagnosticService
                       ├── ASR test
                       ├── LLM test
                       └── TTS test
```

UI lifecycle:

```text
Discover adapter
    ↓
Render typed form
    ↓
Bootstrap-discover options from partial typed selection
    ↓
Select model/voice/language
    ↓
Create/update desired provider
    ↓
Observe runtime_status/requires_restart
    ↓
Restart when required
    ↓
Test loaded provider
    ↓
Bind provider to Template
```

Đây là interface contract cần giữ xuyên suốt implementation:

```text
Descriptor tells UI what an adapter can configure.
Bootstrap discovery tells UI which create-time options are currently selectable.
DB stores what the operator wants.
RuntimeCatalog tells what is actually running.
Diagnostic API tests only what is actually running.
```
