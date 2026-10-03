# 04 — Configuration

## 1. Nguyên tắc

- `config.toml` chứa catalog typed provider instances; `adapter` là implementation compile-time, còn instance ID là binding của agent.
- Environment variables có thể override config runtime khi deployment cần, nhưng không thay đổi source of truth Phase 4 là typed TOML.
- Parse + validate toàn bộ config khi startup; fail fast nếu cấu hình bắt buộc thiếu.
- Session giữ `Arc<AppConfig>` immutable, không đọc file config giữa turn.

## 2. Config mẫu

```toml
[server]
bind = "0.0.0.0:8000"
public_ws_url = "ws://192.168.1.10:8000/voice/v1/"
timezone_offset_minutes = 420
hello_timeout_ms = 5000
shutdown_grace_ms = 5000

[session]
transport_idle_timeout_ms = 300000
conversation_idle_timeout_ms = 0

[auth]
token = ""

[websocket]
max_frame_bytes = 65536

[audio]
input_sample_rate = 16000
output_sample_rate = 24000
channels = 1
frame_ms = 60
uplink_protocol_version = 1
unsupported_protocol_policy = "reject"
ingress_queue_capacity = 128
prebuffer_frames = 3
max_utterance_ms = 30000

[limits]
max_connections = 4
max_active_turns = 2
max_asr_streams = 2
llm_concurrency = 2
tts_concurrency = 2
audio_in_queue = 64
session_event_queue = 64
outbound_control_queue = 32
outbound_audio_queue = 32
urgent_control_queue = 8

[deployment.models]
root = "models"
offline = false

[provider_defaults]
vad = "silero_default"
asr = "zipformer_vi"
llm = "openai_primary"
tts = "zerotts_maichi"

[providers.vad.instances.silero_default]
adapter = "silero_onnx"
model = "silero_v5_16khz"
min_speech_ms = 180
end_silence_ms = 600
pre_roll_ms = 300
speech_threshold = 0.50
exit_threshold = 0.35
sample_rate_hz = 16000
window_samples = 512
num_threads = 1
provider = "cpu"

[barge_in]
enabled = false
trust_client_aec_feature = false

[providers.asr.instances.zipformer_vi]
adapter = "zipformer_sherpa"
model = "zipformer_vi_streaming_chunk32"
timeout_ms = 15000
partial_emit_interval_ms = 200
sample_rate_hz = 16000
num_threads = 2
provider = "cpu"
decoding_method = "greedy_search"
enable_internal_endpoint = false

# Offline Gipformer buffers canonical 16 kHz PCM and decodes it at finish().
# Its instance ID, not its adapter name, is what an agent binding selects.
[providers.asr.instances.gipformer_vi]
adapter = "gipformer_sherpa_offline"
model = "gipformer15_vi_int8"
num_threads = 4
decoding_method = "modified_beam_search" # greedy_search is also supported
max_active_paths = 4

[providers.llm.instances.openai_primary]
adapter = "openai"
api_key = ""
base_url = "https://api.openai.com/v1"
model = "model-name"
timeout_ms = 60000

[llm]
max_history_messages = 20
prompt_budget_tokens = 12000
max_tool_result_chars = 4096

# Caps của Tool-round Executor, áp dụng giống nhau cho Device MCP, External MCP và
# session-local action.
[llm.tools]
max_calls_per_round = 8
max_rounds_per_turn = 4
execution_budget_ms = 30000

# Optional. Omit the whole table to use Mây / vi-VN / built-in template.
[agent]
# name = "Mây"
# language = "vi-VN"
# persona = "Bạn là Mây..."
# prompt_template = "prompts/custom.txt" # relative to this config file

[agent.providers]
# Omit each field to use [provider_defaults].
tts = "chillaudio_default"

[providers.tts.instances.zerotts_maichi]
adapter = "zerotts_onnx"
model = "zerotts_default"
num_threads = 2
voice = "maichi"
delivery_mode = "stream" # stream (default) hoặc file
preload = true

[providers.tts.instances.chillaudio_default]
adapter = "chillaudio_ws"
token = "set-deployment-token-here"
voice = "BV421_vivn_streaming"
preload = false

Startup load mọi instance mà `[provider_defaults]` hoặc `[agent.providers]` tham chiếu, cùng mọi instance có `preload = true`. Instance khác vẫn được parse và validate nhưng chưa có runtime; binding sang một instance chưa load bị reject cho tới phase lazy-load sau này.

[tts]
timeout_ms = 15000

[speech_output]
min_chars = 24
soft_break_min_chars = 48
max_chars = 160
pending_segments = 8

[workers.tts]
max_workers = 2
command_queue_capacity = 32
cleanup_grace_ms = 5000

[mcp]
enabled = true
call_timeout_ms = 30000
# Optional: when omitted, use the non-dangerous catalog announced by the client.
# allowed_tools = ["test.echo", "test.get_value", "test.set_value"]
result_delivery = "llm_then_tts"

# Lookup uses the original MCP name. Direct TTS requires a successful plain-text result.
[[mcp.tool_policy]]
name = "self.audio_speaker.set_volume"
result_delivery = "direct_tts"

# External MCP is server-side Streamable HTTP bound to an Agent, separate from Device MCP.
# Every value here has the shown default; omitting the section is a valid configuration.
[mcp.external]
per_server_resolution_timeout_ms = 3000
overall_resolution_budget_ms = 5000
# Process-global outbound concurrency per MCP server, shared by every session.
max_concurrent_calls_per_server = 16

# Caps apply at the untrusted catalog boundary and reject a server rather than truncate one.
[mcp.external.limits]
max_tools_per_server = 128
max_tools_per_session = 512
max_tool_schema_bytes = 16384
max_tool_description_bytes = 4096
max_external_tool_result_bytes = 16384
max_pages_per_server = 32

# A destination must match an allowlist entry after DNS resolution. HTTPS is the only scheme
# unless allow_http_lan is set for an explicitly LAN-scoped deployment; redirects are off.
[mcp.external.network]
allow_http_lan = false
allowed_hosts = []
allowed_cidrs = []

# Optional Persistent Transcript. Capture is opt-in; retention belongs to the archive rather than
# to capture, so it keeps running and the Admin read/purge stay available when capture is off.
[database.history]
enabled = false
retention_days = 30
queue_capacity = 256

# Unknown-device enrollment requires database-backed admission, Admin API, and auto_register=false.
[database.devices.enrollment]
enabled = false
code_ttl_seconds = 600
retention_seconds = 86400
cleanup_interval_seconds = 60
max_pending = 1000
```

SQLite và WS Device admission là bắt buộc theo [ADR-0073](adr/0073-required-database-and-device-admission.md).
Không còn `database.enabled` hoặc `database.devices.admission_enabled`; xóa hai key cũ
vì strict parser từ chối chúng. Bỏ section database dùng defaults, không tắt DB.
URL/pool/busy-timeout/retention luôn được validate; readiness luôn probe DB.
Admin API/history/enrollment vẫn có cờ riêng. Xem [flow chuyển config và provisioning](flows/08-database-device-enrollment.md).

## 3. Environment override

Khuyến nghị:

```text
VOICE_AGENT_AUTH_TOKEN
VOICE_AGENT_LLM_API_KEY
```

`VOICE_AGENT_LLM_API_KEY` có thể override API key của instance LLM deployment chọn. Dù API key được phép trong TOML, không commit key thật vào repository và không log/debug/telemetry hoặc gửi về client.

## 4. Validation cần có

- V1 input sample rate phải là 16000, output sample rate phải là 24000, channels phải là 1 và frame_ms phải là 60 theo Canonical Audio Profile.
- queue capacity > 0.
- `urgent_control_queue > 0`; urgent lane chỉ cho interrupt `tts:stop`, close và fatal/session control, ưu tiên normal control/audio. Nếu interrupt stop không admission được sau `tts:start`, Voice Session fail-closed; không retry vô hạn.
- `websocket.max_frame_bytes` nằm trong 4.000 bytes–1 MiB và áp dụng chung cho JSON control/MCP lẫn binary audio; frame inbound vượt cap đóng 1009 trước parse/decode. Đây là transport boundary, không phải audio config hay encoder buffer.
- `audio.max_utterance_ms` nằm trong 1.000–120.000 ms và chia hết cho `audio.frame_ms`. Đây là giới hạn chung của Manual Capture và VAD Capture, không phải tham số riêng của VAD; capacity được tính một lần từ integer frame count.
- `unsupported_protocol_policy` V1 chỉ là `reject`; không advertise v2/v3 khi chưa có parser.
- timeout > 0.
- `deployment.models.root` là relative deployment root; Model Artifact Manifest chỉ được dùng install-relative path, reject absolute path, `..` traversal hoặc path escape root. `deployment.models.offline = true` cấm mọi network acquisition.
- mọi instance ID chỉ dùng `[a-zA-Z0-9_-]+`; mỗi instance phải dùng adapter đã build vào binary. `[provider_defaults]` và mọi override trong `[agent.providers]` phải trỏ tới instance đang tồn tại; `AppConfig::load()` materialize effective binding hoàn chỉnh trước startup.
- Typed provider instance chọn adapter và Logical Model Identity, không được chứa direct provider-facing file path. Manifest resolve identity sang source/revision/artifact/transform/checksum; Model Preparation chỉ reuse hoặc acquire/verify/transform/atomic-install trước provider build/warmup và public bind.
- adapter không được tự download model, đoán tên artifact hoặc scan model directory. Provider Factory chỉ nhận Resolved Model theo artifact role sau Model Preparation.
- VAD validate `0.0 <= exit_threshold < speech_threshold <= 1.0`; `min_speech_ms > 0`, `end_silence_ms > 0`, `pre_roll_ms` bounded và retention capacity phải gồm pre-roll, confirmation horizon, bounded VAD in-flight lag cùng rechunk/frame slack.
- `[barge_in]` có hai bool default false. `enabled=true` chỉ có tác dụng khi `trust_client_aec_feature=true`, client Hello có `features.aec=true`, và Listening Mode là Auto/Realtime; đây là client-side echo-suppression assertion, không thay cho server-side AEC.
- `shutdown_grace_ms > 0`; config chỉ có hiệu lực khi process khởi động lại.
- `prompt_budget_tokens > 0`, `max_tool_result_chars > 0`.
- `[llm.tools]` là ba policy cap của Tool-round Executor, validate tại config layer trước khi bind
  listener: `max_calls_per_round ∈ 1..=32`, `max_rounds_per_turn ∈ 1..=8`,
  `execution_budget_ms ∈ 1..=120_000`. Default lần lượt là `8`, `4`, `30_000`. `0` và giá trị vượt
  hard ceiling đều fail startup: đây là policy cap, không phải queue capacity, nên không có queue
  nào được pre-allocate và `SessionActor` không validate lại. `max_rounds_per_turn` đếm các tool
  round mà một Conversational Turn được tiếp tục ngoài round đầu tiên của nó.
- `[agent]` là optional. Field bị omit dùng built-in default; field đã khai báo nhưng rỗng/whitespace fail startup. Built-in template compile vào binary; custom `agent.prompt_template` được resolve một lần theo thư mục config và phải chứa exact `{{persona}}`.
- `llm.max_history_messages > 0`; đây là conversation-history bound, không phải provider adapter config.
- `[database.history]` là policy của optional Persistent Transcript, validate trước khi bind listener: `retention_days ∈ 1..=365`; `queue_capacity ∈ 1..=65_536`. `enabled` quyết định có khởi động `HistoryWriter` hay không, nên khi tắt process không có archival queue hay task nào; `retention_days` được bound bất kể capture đang bật hay tắt vì `RetentionCleaner` sống lâu hơn capture — retention thuộc về archive chứ không thuộc về capture, và tắt capture mới không được biến dữ liệu đã có thành retention vô hạn. `0` bị từ chối như giá trị vượt range, vì một retention không có cửa sổ không phải retention policy. Default lần lượt là `false`, `30`, `256`.
- `[database.devices.enrollment]` mặc định tắt. Khi bật, `api.enabled` bắt buộc true; database và Device admission luôn hoạt động, còn `auto_register` bắt buộc false. V1 giới hạn TTL `60..=3600` giây, retention `TTL..=604800` giây, cleanup interval `10..=3600` giây và pending capacity `1..=10000`; OTA cấp mã không bao giờ gia hạn TTL khi thiết bị poll lại.
- mỗi instance OpenAI phải có `base_url`, `model`, timeout hợp lệ; API key có thể nằm TOML nhưng không xuất hiện trong `Debug`, error, log hay telemetry. Nhiều instance có thể cùng adapter `openai` với endpoint/model khác nhau.
- mỗi instance ZeroTTS phải có Logical Model Identity, `voice` và thread count hợp lệ; Model Preparation inject `ResolvedModel`, không direct path. Remote ChillAudio không có fake model identity. `[workers.tts]` là template capacity/timeout/cleanup cho từng runtime được load.
- `gipformer_sherpa_offline` dùng `OfflineRecognizer`: `push_pcm()` chỉ tích luỹ PCM 16 kHz canonical, không phát partial; `finish()` mới decode toàn utterance. `model`, `num_threads`, `decoding_method` (`greedy_search` hoặc `modified_beam_search`) và `max_active_paths > 0` là các option duy nhất của instance. Precision và artifact paths thuộc Model Artifact Manifest, không thuộc TOML provider. Offline decode hiện không hard-cancel được sau khi native decode bắt đầu, nên giữ `workers.asr.final_timeout_ms` theo benchmark target CPU.
- `[speech_output]` chứa `min_chars`, `soft_break_min_chars`, `max_chars`, `pending_segments` với `1 <= min_chars <= soft_break_min_chars <= max_chars`, `1 <= pending_segments <= 64`. Hai ngưỡng tối thiểu chỉ được giữ để tương thích cấu hình cũ; dấu kết câu flush ngay, dấu mềm không flush. `max_chars` chỉ là ngưỡng khẩn cấp: buffer câu chưa hoàn tất vượt `2 * max_chars` sẽ fail backpressure, không bị cắt giữa câu. Pending full cũng fail `speech_output_backpressure`, cancel LLM operation và không accept thêm delta.
- `tts.timeout_ms` bắt đầu khi TtsWorkerRuntime accept một segment và kết thúc tại `SegmentFinished`, `Failed` hoặc cancelled acknowledgement; PCM chunks không reset timer. Slot chỉ release sau cleanup acknowledgement, hoặc worker bị quarantine khi hết cleanup grace.
- `llm_concurrency > 0`; LlmRuntime giữ permit từ khi accept request tới terminal event, timeout request không reset bởi text delta. Bounded event route không được drop terminal event; failure route phải cancel controlled operation.
- `limits.tts_concurrency` là global admission budget; `workers.tts.max_workers` là structural capacity của từng loaded TTS runtime, nên hai giá trị không còn bắt buộc bằng nhau.
- OpenAI startup chỉ validate local typed config/build provider; không model list, completion, health probe hay network request trước bind. Lỗi remote thuộc LLM Operation hiện tại.
- acknowledgement của `zerotts_default` match chính xác `license = "MIT; bundled-codec=Apache-2.0"`; Phase 4 giữ license model-level, nhưng `codec_license` vẫn là required artifact.
- public WS URL luôn phải hợp lệ vì OTA discovery luôn có.
- `auth.token = ""` tắt authentication; token không rỗng bắt buộc Bearer token. Device-Id và Client-Id không phải credential.
- OTA chỉ trả static token khi auth bật và Device/Agent được phép trong DB; Device-Id tự khai chưa phải device credential. Internet không nằm trong supported V1 profile.
- mọi limits và queue capacity > 0.
- `[mcp.external]` toàn bộ field có default, nên bỏ hẳn section cũng là configuration hợp lệ. `per_server_resolution_timeout_ms` và `overall_resolution_budget_ms` phải dương và `overall_resolution_budget_ms >= per_server_resolution_timeout_ms`; `max_concurrent_calls_per_server` nằm trong `1..=64` và đây chính là bound của process-global semaphore per MCP server dùng chung cho mọi session. Operator chỉ cấu hình trong hard ceiling: `max_tools_per_server <= 512`, `max_tools_per_session <= 2_048`, `max_tool_schema_bytes` và `max_external_tool_result_bytes <= 65_536`, `max_tool_description_bytes <= 16_384`, `max_pages_per_server <= 256`. Mọi limit phải dương, và một tổ hợp limit mô tả một `tools/list` page lớn hơn buffer một response thì fail startup thay vì biến thành server lặng lẽ không resolve được.
- `[mcp.external.network]` bắt buộc có ít nhất một entry trong `allowed_hosts` hoặc `allowed_cidrs`; chỉ HTTPS trừ khi `allow_http_lan = true`, và khi đó destination vẫn phải match allowlist. `allowed_hosts` chỉ nhận hostname pattern hợp lệ và `allowed_cidrs` chỉ nhận CIDR hợp lệ. URL không có userinfo, query string hay fragment; redirect tắt. Validate hostname allowlist, resolve DNS ngay trước connect, và mọi resolved IP cũng phải pass policy để chống DNS rebinding.

