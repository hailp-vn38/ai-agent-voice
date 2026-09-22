# 04 — Configuration

## 1. Nguyên tắc

- `config.toml` cho non-secret config.
- Environment variables override secret/config runtime.
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

[deployment.models]
root = "models"
offline = false

[providers.vad]
adapter = "silero_onnx"

[providers.vad.silero_onnx]
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

[providers.asr]
adapter = "zipformer_sherpa"

[providers.asr.zipformer_sherpa]
model = "zipformer_vi_streaming_chunk32"
timeout_ms = 15000
partial_emit_interval_ms = 200
sample_rate_hz = 16000
num_threads = 2
provider = "cpu"
decoding_method = "greedy_search"
enable_internal_endpoint = false

[llm]
adapter = "openai_chat_completions_v1"
base_url = "https://api.example.com/v1"
model = "model-name"
timeout_ms = 60000
max_history_messages = 20
prompt_budget_tokens = 12000
max_tool_result_chars = 4096
max_tool_depth = 4

[tts]
adapter = "openai_speech_v1"
base_url = "http://127.0.0.1:9002"
model = "gpt-4o-mini-tts"
voice = "default"
timeout_ms = 15000

[mcp]
enabled = true
call_timeout_ms = 30000
allowed_tools = ["self.get_device_status", "self.audio_speaker.set_volume"]
```

## 3. Environment override

Khuyến nghị:

```text
VOICE_AGENT_AUTH_TOKEN
VOICE_AGENT_LLM_API_KEY
VOICE_AGENT_TTS_API_KEY
```

Không commit API key vào repository.

## 4. Validation cần có

- V1 input sample rate phải là 16000, output sample rate phải là 24000, channels phải là 1 và frame_ms phải là 60 theo Canonical Audio Profile.
- queue capacity > 0.
- `websocket.max_frame_bytes` nằm trong 4.000 bytes–1 MiB và áp dụng chung cho JSON control/MCP lẫn binary audio; frame inbound vượt cap đóng 1009 trước parse/decode. Đây là transport boundary, không phải audio config hay encoder buffer.
- `audio.max_utterance_ms` nằm trong 1.000–120.000 ms và chia hết cho `audio.frame_ms`. Đây là giới hạn chung của Manual Capture và VAD Capture, không phải tham số riêng của VAD; capacity được tính một lần từ integer frame count.
- `unsupported_protocol_policy` V1 chỉ là `reject`; không advertise v2/v3 khi chưa có parser.
- timeout > 0.
- `deployment.models.root` là relative deployment root; Model Artifact Manifest chỉ được dùng install-relative path, reject absolute path, `..` traversal hoặc path escape root. `deployment.models.offline = true` cấm mọi network acquisition.
- adapter phải được build vào binary. Typed provider config chọn adapter và Logical Model Identity, không được chứa direct provider-facing file path. Manifest resolve identity sang source/revision/artifact/transform/checksum; Model Preparation chỉ reuse hoặc acquire/verify/transform/atomic-install trước provider build/warmup và public bind.
- adapter không được tự download model, đoán tên artifact hoặc scan model directory. Provider Factory chỉ nhận Resolved Model theo artifact role sau Model Preparation.
- VAD validate `0.0 <= exit_threshold < speech_threshold <= 1.0`; `min_speech_ms > 0`, `end_silence_ms > 0`, `pre_roll_ms` bounded và retention capacity phải gồm pre-roll, confirmation horizon, bounded VAD in-flight lag cùng rechunk/frame slack.
- `shutdown_grace_ms > 0`; config chỉ có hiệu lực khi process khởi động lại.
- `prompt_budget_tokens > 0`, `max_tool_result_chars > 0`, `max_tool_depth > 0`.
- `llm.max_history_messages > 0`; đây là conversation-history bound, không phải provider adapter config.
- public WS URL hợp lệ nếu OTA được bật.
- `auth.token = ""` tắt authentication; token không rỗng bắt buộc Bearer token. Device-Id và Client-Id không phải credential.
- OTA trả static token khi auth bật và không phải security boundary; Internet không nằm trong supported V1 profile.
- mọi limits và queue capacity > 0.
