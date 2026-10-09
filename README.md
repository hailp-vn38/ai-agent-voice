# voice-agent-server

Voice-agent server viết bằng Rust, phục vụ client phần mềm hoặc thiết bị tuân thủ **WebSocket Voice Protocol V1**. Server điều phối nhận giọng nói, gọi LLM và phát câu trả lời bằng Opus; **Voice Agent Studio** là giao diện quản trị Vue tùy chọn.

Firmware reference là nguồn đối chiếu tương thích đã pin, không phải loại client duy nhất. Dự án định nghĩa wire contract và hành vi riêng, không port reference implementation theo kiểu 1:1.

## Chức năng hiện có

- OTA/discovery và WebSocket V1; Opus uplink 16 kHz, downlink 24 kHz, mono, frame 60 ms.
- VAD, các listening mode Manual/Auto/Realtime, ASR, streaming LLM và TTS có pacing.
- Dialogue History trong RAM, abort/cancellation và acoustic barge-in khi bật policy tin cậy client có AEC.
- SQLite control plane bắt buộc: Agent, Template, Provider và Device admission cho mọi kết nối voice.
- Device enrollment bằng mã sáu chữ số qua WebSocket hoặc OTA.
- Device MCP và External MCP qua Streamable HTTP; External MCP cần Agent binding và duyệt từng tool.
- Admin API có bearer token riêng; quản lý cấu hình, credential mã hóa và transcript lưu trữ tùy chọn.
- Speaker recognition CAM++ ở chế độ `off`/`observe`: kết quả chỉ hỗ trợ cá nhân hóa từng lượt, không xác thực hay cấp quyền.
- Vision tùy chọn và kiểm thử Provider/MCP từ Admin Web.

Core V1 không bắt buộc Admin Web, multi-user, MQTT/UDP gateway, RAG, billing/quota, plugin hot-load hay long-term memory. Speaker policy `required` và Independent Confirmation chưa triển khai.

## Cấu trúc dự án

| Đường dẫn | Vai trò |
| --- | --- |
| [`crates/voice-agent-server/`](crates/voice-agent-server/) | Rust server, provider adapters, workers, session actor và tests |
| [`apps/admin-web/`](apps/admin-web/) | Voice Agent Studio: Vue 3, TypeScript, Vite, Pinia, Tailwind CSS |
| [`config.example.toml`](config.example.toml) | Cấu hình deployment mẫu |
| [`CONTEXT.md`](CONTEXT.md) | Thuật ngữ và domain model |
| [`docs/`](docs/) | Flow, API, chiến lược test và ADR |
| [`prompts/`](prompts/) | Prompt templates |
| [`assets/enrollment/`](assets/enrollment/) | Audio hướng dẫn enrollment |
| [`benchmarks/`](benchmarks/) | Workload và audio fixtures cho benchmark |
| [`scripts/`](scripts/) | Kiểm thử, chuẩn bị assets và quản lý Docker Compose |
| [`docker/`](docker/) | Dockerfiles, nginx và cấu hình deployment mẫu |

## Yêu cầu môi trường

- Rust theo [`rust-toolchain.toml`](rust-toolchain.toml) (hiện pin `1.98.1`), C/C++ toolchain, CMake và pkg-config để build các thư viện native. Libopus được bundled khi build.
- ONNX Runtime dynamic library cho provider ONNX local; cấu hình đường dẫn trong `[runtime.onnx].library`. Mẫu hiện dùng `.dylib` cho macOS; Linux cần đường dẫn tới `libonnxruntime.so` tương ứng. Docker production cài ONNX Runtime trong image.
- Credential và endpoint hợp lệ cho provider remote được chọn. Kokoro VI local cần G2P executable riêng.
- Node.js **22.12 trở lên** và npm nếu chạy Admin Web.
- Python 3, `espeak-ng` và `ffmpeg` nếu tạo audio enrollment bằng script.

Chạy các lệnh backend từ thư mục gốc repository. [Cấu hình Cargo](.cargo/config.toml) hiện đặt build output tại `/mnt/storage/ai-agent-voice/target`; trên máy khác có thể dùng:

```bash
export CARGO_TARGET_DIR="$PWD/target"
```

## Chạy local

### 1. Chuẩn bị cấu hình

Nếu chưa có `config.toml`, sao chép mẫu:

```bash
cp config.example.toml config.toml
```

Chỉnh `config.toml` trước khi chạy:

- `[provider_defaults]` chọn instance VAD/ASR/LLM/TTS đã khai báo trong `[providers.<kind>.instances.<id>]`. Mẫu hiện chọn Silero, Gipformer, OpenAI và ChillAudio.
- Điền LLM `api_key`, `base_url`, `model` và TTS token/endpoint phù hợp; giá trị placeholder trong mẫu chưa đủ để trò chuyện.
- Sửa `[runtime.onnx].library` theo hệ điều hành và thư viện đã cài.
- Đặt `[api].enabled = true` và `admin_token` riêng, không rỗng để dùng Studio hoặc provision qua Admin API.
- `[auth].token` là bearer token cho voice, độc lập với admin token. Giá trị rỗng tắt voice bearer authentication nhưng vẫn yêu cầu Device admission.

SQLite mặc định nằm tại `data/voice-agent.db`. Server tự tạo thư mục cha và chạy migrations. SQLite V1 dành cho **một process trên filesystem local**; không dùng NFS/SMB hoặc nhiều process cùng ghi. Hai key cũ `database.enabled` và `database.devices.admission_enabled` không còn được chấp nhận.

### 2. Khởi động server

```bash
VOICE_AGENT_CONFIG=config.toml cargo run --locked -p voice-agent-server --bin voice-agent-server
```

Nếu không đặt `VOICE_AGENT_CONFIG`, binary đọc `config.toml`. Endpoint mặc định:

| Endpoint | Mục đích |
| --- | --- |
| `GET /health` | Liveness |
| `GET /ready` | Readiness, bao gồm kiểm tra SQLite |
| `/voice/ota/` | Discovery/cấu hình kết nối cho Device |
| `ws://127.0.0.1:8000/voice/v1/` | Voice Protocol V1 |
| `/api/admin/*` | Quản trị, chỉ mount khi `api.enabled = true` |

Kiểm tra readiness sau khi server bind:

```bash
curl --fail http://127.0.0.1:8000/ready
```

Startup chuẩn bị model của **mọi provider local khai báo trong TOML**, kể cả provider không mặc định. File thiếu được tải vào `models/<KIND>/<provider>`; file đã có và khác 0 byte được dùng lại, không kiểm checksum. Provider remote không tải model. ZeroTTS chuẩn bị toàn bộ voice được hỗ trợ.

Sau đó server dựng runtime của các provider mặc định và TTS có `preload = true`; provider khác được materialize khi cần. Lần chạy đầu có thể tải model lớn và mất nhiều thời gian. Không cài được assets hoặc không dựng được runtime bắt buộc sẽ làm startup fail trước khi phục vụ request. Xem [quyết định về model assets](docs/adr/0076-provider-owned-model-assets.md).

### 3. Chạy Voice Agent Studio và provision Device

Ở terminal khác:

```bash
cd apps/admin-web
# Chỉ sao chép khi chưa có .env riêng.
cp .env.example .env
npm ci
npm run dev
```

Mở `http://127.0.0.1:5173` và nhập admin bearer token vào form kết nối. Vite proxy tới `http://127.0.0.1:8000`; đổi `VITE_DEV_PROXY_TARGET` trong `.env` nếu backend ở địa chỉ khác. Nhập token qua form; `VITE_ADMIN_TOKEN`, nếu cấu hình, sẽ được đưa vào browser bundle.

Trong Studio, tạo Agent rồi thêm Device với Device ID đúng như client gửi. Có thể liên kết Template và cấu hình provider bindings; slot chưa được bind dùng server defaults. Agent và Device phải enabled. Database trống không tự tạo Agent/Device; khi enrollment tắt, Device chưa đăng ký bị từ chối OTA/WS với HTTP 403.

Studio quản lý Agents, Templates, Providers, Devices, External MCP, Speakers và trạng thái System. Provider draft inference và MCP connection/discovery giúp kiểm thử trước khi lưu; kết quả test không tự duyệt tool hay thay đổi Agent bindings. Provider draft inference cần Provider Runtime Manager và memory estimate cho adapter. Xem [Admin Web](apps/admin-web/README.md), [manual testing API](docs/api/live-testing.md) và [Postman collection](docs/api/00-all-apis.postman_collection.json).

## Credential Provider/MCP

Để lưu token từ Admin Web/API, backend cần khóa mã hóa AES-256-GCM, đặt **riêng với SQLite**. Chọn một trong hai cách:

- File JSON qua `[deployment].credential_keys_file`, gồm `current_version` và `keys` ánh xạ phiên bản sang khóa Base64 32 byte.
- Biến môi trường `VOICE_CREDENTIAL_KEY_VERSION` (mặc định `1`) và `VOICE_CREDENTIAL_KEY_<version>` chứa khóa Base64 32 byte.

Ví dụ cấu trúc file khóa, thay placeholder bằng khóa riêng được tạo bằng `openssl rand -base64 32`:

```json
{
  "current_version": 1,
  "keys": {
    "1": "<base64-encoded-32-byte-key>"
  }
}
```

File được nạp khi khởi động và ưu tiên hơn khóa môi trường. Giữ khóa qua restart; giữ cả phiên bản cũ khi còn bản ghi dùng chúng. Mất khóa khiến credential đã lưu không dùng được. Không commit khóa/token; bảo vệ database, backup và file khóa. Chi tiết contract nằm trong [ADR 0083](docs/adr/0083-admin-managed-resource-credentials.md).

## Device enrollment

Enrollment là opt-in, cần Admin API bật và `database.devices.auto_register = false`. Với WebSocket enrollment, chuẩn bị 11 WAV tiếng Việt trước:

```bash
python3 scripts/prepare-enrollment-assets.py
```

Sau đó đặt các giá trị sau trong bảng `[database.devices.enrollment]` hiện có:

```toml
[database.devices.enrollment]
enabled = true
transport = "websocket"
prompt_assets_dir = "assets/enrollment/vi-VN"
```

Device chưa đăng ký có Enrollment Session riêng để hiển thị và phát mã sáu chữ số; kết nối này không chạy conversational providers, MCP hoặc transcript. Claim mã qua Admin API/Studio để gán Device vào Agent. Sau claim, client cần kết nối WS lại để bắt đầu Voice Session. Missing/invalid audio assets làm startup fail. Đặt `transport = "ota"` để dùng activation/polling thay vì WS onboarding; xem [ADR 0074](docs/adr/0074-websocket-enrollment-session.md).

## MCP, transcript và giới hạn triển khai

- **Device MCP:** khi `mcp.enabled = true`, các tool hợp lệ, không mơ hồ từ discovery hoàn chỉnh được công bố cho Voice Session của chính Device đó.
- **External MCP:** khai báo server, bind vào Agent, thực hiện discovery rồi duyệt tool theo contract quan sát được. Discovery không tự cấp quyền; tool nhạy cảm vẫn bị chặn vì chưa có Independent Confirmation. Xem [tool allowlist](docs/api/tool-allowlist.md).
- **Transcript:** `[database.history].enabled = false` theo mặc định. Bật capture để lưu transcript; tắt capture không tắt retention của dữ liệu đã lưu. Dialogue History của Voice Session vẫn ở RAM.
- **Speaker:** `observe` chỉ điều chỉnh prompt của lượt hiện tại. Bật `[deployment].speaker_pilot = true` để giới hạn một voice pipeline xử lý trong toàn process; các socket idle vẫn có thể kết nối.
- **Network:** V1 hướng tới trusted LAN. Khi cấu hình client ở máy khác, sửa `server.bind` và `server.public_ws_url` theo địa chỉ thực tế. Triển khai qua Internet cần WSS/reverse proxy/VPN và token; Provider credential submission trong production cần HTTPS termination.

## Kiểm thử

Gate backend hiện có:

```bash
./scripts/test-all.sh
```

Script chạy `cargo fmt --check`, Clippy trên mọi target/feature và `cargo test`. Kiểm thử qualification providers riêng:

```bash
cargo test --locked --workspace --features qualification-providers
```

Kiểm thử/build Admin Web:

```bash
cd apps/admin-web
npm test
npm run build
```

`npm run build` bao gồm typecheck. Các gate dùng model/provider thật là opt-in; không coi test deterministic là bằng chứng đã kiểm tra hardware hay API thật. Xem [chiến lược test](docs/testing/00-test-strategy.md).

## Docker

Repository có Compose production và qualification/test; nginx phục vụ Studio và proxy API/WS đến backend. Dùng wrapper từ thư mục gốc:

```bash
bash scripts/compose.sh --help
bash scripts/compose.sh prod config
bash scripts/compose.sh prod build
bash scripts/compose.sh prod up
```

Trước khi chạy production, tạo `docker/config.prod.toml` và `docker/secrets.env` từ [config mẫu](docker/config.prod.example.toml) và [env mẫu](docker/secrets.env.example), chỉnh provider credentials, bind và runtime library. Chuẩn bị thư mục data/models và quyền ghi theo [`compose.prod.yaml`](compose.prod.yaml); web mặc định ở `http://127.0.0.1:8080`.

Compose và storage checker hiện gắn với host có storage tại `/mnt/storage/ai-agent-voice`; cần đối chiếu [storage checker](scripts/check-docker-storage.py) và volume paths khi dùng máy khác. Backend production không publish cổng trực tiếp ra host.

```bash
bash scripts/compose.sh test run
```

Lệnh trên chạy checks và smoke tests trong môi trường test riêng, **xóa test data** khi teardown. Không dùng dữ liệu production cho test stack.

## Provider benchmarks

`provider-bench-av` đo boundary ASR/VAD/LLM, dùng workload đã version control; warmup không tính vào số liệu. LLM đo TTFT tại `TextDelta` không rỗng đầu tiên, total latency và chunk/tool-call counts.

```bash
cargo run --locked --release -p voice-agent-server --bin provider-bench-av -- \
  llm --config config.toml \
  --workload benchmarks/performance_tester/workloads/llm-v1.json \
  --warmup 1 --iterations 5 --output target/benchmarks/llm.json

cargo run --locked --release -p voice-agent-server --bin provider-bench-av -- \
  asr --config config.toml \
  --workload benchmarks/performance_tester/workloads/asr-v1.json \
  --feed burst --warmup 1 --iterations 5

cargo run --locked --release -p voice-agent-server --bin provider-bench-av -- \
  vad --config config.toml \
  --workload benchmarks/performance_tester/workloads/vad-v1.json \
  --warmup 1 --iterations 5
```

TTS dùng `provider-bench`, với mode `provider` đo tới PCM hoặc `delivery` đo tới Opus packet sẵn sàng gửi:

```bash
cargo run --locked --release -p voice-agent-server --bin provider-bench -- \
  tts provider --config config.toml --warmup-runs 1 --runs 5
```

Các benchmark dùng provider được chọn trong cấu hình và có thể tải model/gọi dịch vụ thật. Report không ghi API key; thời gian phụ thuộc host, model và execution settings.

## Thứ tự đọc tài liệu

1. [Domain model và thuật ngữ](CONTEXT.md).
2. [Các flow voice pipeline](docs/flows/README.md).
3. [Structured system prompt](docs/structured-voice-system-prompt.md).
4. [API collection](docs/api/00-all-apis.postman_collection.json), [Provider/MCP testing](docs/api/live-testing.md), [tool allowlist](docs/api/tool-allowlist.md).
5. [Chiến lược và contract tests](docs/testing/README.md).
6. [Quyết định kiến trúc](docs/adr/).
