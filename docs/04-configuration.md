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
public_ws_url = "ws://192.168.1.10:8000/xiaozhi/v1/"
timezone_offset_minutes = 420
hello_timeout_ms = 5000
shutdown_grace_ms = 5000

[session]
transport_idle_timeout_ms = 300000
conversation_idle_timeout_ms = 0

[auth]
token = ""

[audio]
input_sample_rate = 16000
output_sample_rate = 24000
channels = 1
frame_ms = 60
uplink_protocol_version = 1
unsupported_protocol_policy = "reject"
max_ws_frame_bytes = 8192
ingress_queue_capacity = 128
prebuffer_frames = 3

[limits]
max_connections = 4
max_active_turns = 2
asr_concurrency = 2
llm_concurrency = 2
tts_concurrency = 2
audio_in_queue = 64
session_event_queue = 64
outbound_control_queue = 32
outbound_audio_queue = 32

[vad]
provider = "local"
min_speech_ms = 180
end_silence_ms = 600
pre_roll_ms = 300
max_utterance_ms = 30000

[asr]
adapter = "openai_transcription_v1"
base_url = "http://127.0.0.1:9001"
model = "gpt-4o-mini-transcribe"
timeout_ms = 15000

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
XIAOZHI_AUTH_TOKEN
XIAOZHI_ASR_API_KEY
XIAOZHI_LLM_API_KEY
XIAOZHI_TTS_API_KEY
```

Không commit API key vào repository.

## 4. Validation cần có

- V1 input sample rate phải là 16000, output sample rate phải là 24000, channels phải là 1 và frame_ms phải là 60 theo Canonical Audio Profile.
- queue capacity > 0.
- `max_ws_frame_bytes`, `max_utterance_ms`, hello/idle timeout đều > 0; frame text hoặc binary vượt `max_ws_frame_bytes` đóng 1009 trước parse/decode.
- `unsupported_protocol_policy` V1 chỉ là `reject`; không advertise v2/v3 khi chưa có parser.
- timeout > 0.
- `shutdown_grace_ms > 0`; config chỉ có hiệu lực khi process khởi động lại.
- `prompt_budget_tokens > 0`, `max_tool_result_chars > 0`, `max_tool_depth > 0`.
- public WS URL hợp lệ nếu OTA được bật.
- `auth.token = ""` tắt authentication; token không rỗng bắt buộc Bearer token. Device-Id và Client-Id không phải credential.
- OTA trả static token khi auth bật và không phải security boundary; Internet không nằm trong supported V1 profile.
- mọi limits và queue capacity > 0.
