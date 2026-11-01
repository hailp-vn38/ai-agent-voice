# voice-agent-server

Trợ lý giọng nói và tác vụ nền viết bằng Rust, phục vụ client phần mềm hoặc thiết bị tuân thủ **WebSocket Voice Protocol V1**. Server điều phối ASR → LLM → TTS, quản lý tác vụ bằng agent, Knowledge Wiki và Memory theo Device. **Voice Agent Studio** là giao diện quản trị Vue, kèm Studio Assistant để trò chuyện ngay trong trình duyệt.

Firmware reference là nguồn đối chiếu tương thích đã pin, không phải loại client duy nhất. Dự án định nghĩa wire contract và hành vi riêng, không port reference implementation theo kiểu 1:1.

## Chức năng hiện có

- **Voice:** OTA/discovery, WebSocket V1, Opus uplink 16 kHz/downlink 24 kHz, mono, frame 60 ms; VAD, Manual/Auto/Realtime, ASR, streaming LLM, TTS có pacing, abort và acoustic barge-in theo policy AEC.
- **Control plane:** SQLite quản lý Agents, Templates, Providers, Devices, enrollment bằng mã sáu chữ số, transcript tùy chọn và credential mã hóa.
- **Studio Assistant:** Voice Session trên Virtual Device, dùng cùng voice pipeline; có System MCP tùy chọn cho các công cụ quản trị chỉ đọc.
- **Jobs:** tác vụ nền và tác vụ theo lịch, follow-up, phân công sub-agent, catalog Skills, lịch sử thực thi, báo cáo và artifacts. Runtime dùng `orchestrator-flow` và `orchestrator-agent`.
- **Knowledge:** Wiki Markdown qua `llm-wiki-rs`, tìm kiếm, liên kết/đồ thị và Wiki Auto-Flow xử lý nguồn mới; có Knowledge MCP cho client agent.
- **Memory:** Memory Vault riêng theo Device, đọc/ghi qua `MemoryCore`; mặc định tắt, chỉ có hiệu lực khi bật cả deployment và Device.
- **LLM Gateway:** quản lý OAuth/tài khoản, pool, model và quota cho Anthropic, OpenAI Codex, Gemini, GitHub Copilot và Muse; dùng qua provider binding nội bộ.
- **Thiết bị và vận hành:** Device MCP, External MCP với duyệt tool, reminders, notification channels, log có cấu trúc, giám sát runtime và backup/import database.
- **Provider:** adapter local/remote, Vision tùy chọn, speaker recognition CAM++ `off`/`observe` và kiểm thử Provider/MCP từ Studio.

Speaker recognition chỉ hỗ trợ cá nhân hóa, không xác thực hay cấp quyền. Speaker policy `required` và Independent Confirmation chưa triển khai. Firmware reference là nguồn đối chiếu tương thích; chạy deterministic tests không thay thế kiểm chứng thiết bị, Docker hoặc provider thật.

## Cấu trúc dự án

| Đường dẫn | Vai trò |
| --- | --- |
| [`crates/voice-agent-server/`](crates/voice-agent-server/) | Rust server, provider adapters, workers, session actor và tests |
| [`apps/admin-web/`](apps/admin-web/) | Voice Agent Studio: Vue 3, TypeScript, Vite, Pinia, Tailwind CSS |
| [`config.example.toml`](config.example.toml) | Cấu hình deployment mẫu |
| [`CONTEXT.md`](CONTEXT.md) | Thuật ngữ và domain model |
| [`docs/`](docs/) | Flow, API, chiến lược test và ADR |
| [`vendor/orchestrator_agent_rs/`](vendor/orchestrator_agent_rs/) | Agent runtime, tool execution và recovery |
| [`vendor/orchestrator_task_rs/`](vendor/orchestrator_task_rs/) | TaskManager, lịch và Wiki Auto-Flow (`orchestrator-flow`) |
| [`vendor/llm-wiki-rs/`](vendor/llm-wiki-rs/) | WikiEngine, Markdown vault và tìm kiếm |
| [`prompts/`](prompts/) | Prompt templates cho voice và agent |
| [`skills/`](skills/) | Skills Markdown cho tác vụ nền |
| [`assets/enrollment/`](assets/enrollment/) | Audio hướng dẫn enrollment |
| [`benchmarks/`](benchmarks/) | Workload và audio fixtures cho benchmark |
| [`scripts/`](scripts/) | Wrapper kiểm tra, chuẩn bị assets và quản lý Docker Compose |
| [`docker/`](docker/) | Dockerfiles, nginx và cấu hình deployment mẫu |

## Yêu cầu môi trường

- Rust theo [`rust-toolchain.toml`](rust-toolchain.toml) (hiện pin `1.98.1`), C/C++ toolchain, CMake và pkg-config để build các thư viện native. Libopus được bundled khi build.
- ONNX Runtime dynamic library cho provider ONNX local; cấu hình đường dẫn trong `[runtime.onnx].library`. Mẫu hiện dùng `.dylib` cho macOS; Linux cần đường dẫn tới `libonnxruntime.so` tương ứng. Docker production cài ONNX Runtime trong image.
- Credential và endpoint hợp lệ cho provider remote được chọn. Kokoro VI local cần G2P executable riêng.
- Node.js **22.12 trở lên** và npm nếu chạy Admin Web.
- Python 3, `espeak-ng` và `ffmpeg` nếu tạo audio enrollment bằng script.

Chạy các lệnh backend từ thư mục gốc repository. Root workspace chỉ gồm `voice-agent-server`; ba thư viện trong `vendor/` được dùng qua `[patch.crates-io]` và có workspace riêng. Giữ đầy đủ các thư mục này khi checkout/build. Nếu cần chọn thư mục build riêng:

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
- Mẫu bật `[orchestrator].enabled = true`: cấu hình `endpoint`, `api_key`, `model` cho tác vụ nền. Cấu hình LLM này độc lập với provider voice mặc định. Đặt `enabled = false` nếu chỉ chạy voice.
- `[wiki]`, `[wiki.mcp]`, `[memory]` và `[system_mcp]` là các phần bật riêng; không bật tất cả chỉ để khởi động voice.

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

Sau đó server dựng runtime của các provider mặc định, Vision khi bật và TTS có `preload = true`; Provider Runtime Manager hỗ trợ materialize provider khi cần nếu được cấu hình. Lần chạy đầu có thể tải model lớn và mất nhiều thời gian. Không cài được assets hoặc không dựng được runtime bắt buộc sẽ làm startup fail trước khi phục vụ request. Xem [quyết định về model assets](docs/adr/0076-provider-owned-model-assets.md).

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

Để dùng Studio Assistant trong trình duyệt, vào System → Assistant, chọn Agent/Template và bật Virtual Device. Trình duyệt cần quyền microphone; khi truy cập từ máy khác, dùng HTTPS để các API microphone hoạt động. Xem [contract Studio Assistant](docs/api/studio-assistant.md).

Studio quản lý Agents, Templates, Providers, Devices, External MCP, Speakers, Jobs, Skills/sub-agents, Knowledge, Memory, LLM Gateway, Notifications và System. Provider draft inference và MCP connection/discovery giúp kiểm thử trước khi lưu; kết quả test không tự duyệt tool hay thay đổi Agent bindings. Provider draft inference cần Provider Runtime Manager và memory estimate cho adapter. Xem [Admin Web](apps/admin-web/README.md), [manual testing API](docs/api/live-testing.md) và [Postman collection](docs/api/00-all-apis.postman_collection.json).

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

## Tác vụ nền, Knowledge và Memory

### Jobs và Skills

`[orchestrator]` cấp cấu hình cho `TaskManager` và agent runtime. Trong Studio, dùng **Jobs** để tạo tác vụ, theo dõi cây thực thi, gửi follow-up, xem báo cáo/artifacts và quản lý cấu hình providers, agents, Skills.

- `daily_root` điều phối tác vụ tương tác, thu thập nguồn và báo cáo; `wiki_root` phụ trách biên soạn/duy trì Wiki từ nguồn local.
- WorkspaceToolKit cung cấp công cụ Wiki, báo cáo Markdown, HTML/PDF/YouTube, media và reminders theo quyền từng agent. Backend ảnh/video, ASR/TTS và CLI ngoài cần được chuẩn bị cho công cụ tương ứng; chẳng hạn YouTube cần `yt-dlp` trên `PATH`.
- Runtime có trạng thái chờ retry/chờ người dùng và ngân sách mỗi lượt; tác vụ có side effect không rõ kết quả không được tự động chạy lại như một yêu cầu mới. Chi tiết: [ADR 0133](docs/adr/0133-durable-waiting-retry-and-job-budgets.md), [ADR 0134](docs/adr/0134-resumable-tool-operations-and-job-muse-sessions.md).

Runtime hiện tại thay thế Job engine và plugin runner cũ: không còn quy trình build/upload/activate plugin ZIP hay các route `/api/admin/plugins*`, `/api/admin/core-plugins*`. Các route `/api/admin/jobs*` và `/api/admin/background/settings` tiếp tục phục vụ Studio. Xem [ADR 0127](docs/adr/0127-orchestrator-flow-task-and-wiki-runtime.md), [catalog Skills](docs/adr/0128-admin-managed-job-skill-catalog.md) và [sub-agents](docs/adr/0129-admin-managed-subagent-catalog.md).

### Knowledge Wiki

`[wiki].vault_dir` chọn vault Markdown. Bật `[wiki].enabled` để dùng Wiki độc lập; khi Orchestrator bật, runtime cũng mở WikiEngine để phục vụ toolkit. Wiki Auto-Flow theo dõi nguồn Markdown mới trong `sources/`, biên soạn sang `wiki/`, cập nhật tổng hợp và chạy lint.

Để client agent truy cập Knowledge, bật `[wiki.mcp].enabled` và đặt token riêng (`token` hoặc `read_token`/`write_token`). Endpoint mặc định là `/mcp/knowledge`, hỗ trợ `Authorization: Bearer <token>` hoặc `X-API-Key`. Cấu hình quyền ghi qua `allow_write`; Knowledge MCP không dùng admin bearer token thay cho token riêng.

### Memory theo Device

Bật `[memory].enabled = true`, chọn `vault_dir`, sau đó chọn `memory_mode` cho từng Device trong Studio: `off` (tắt), `read` (chỉ đọc), `read_write` (đọc và ghi). Memory Vault tách khỏi Wiki; Memory Fact được lưu có chủ đích có thể đi vào prompt của LLM. Phân vùng theo Device không thay thế xác thực trên mạng không tin cậy.

Thao tác quên chỉ xử lý dữ liệu trong Memory Vault; không xóa Dialogue History, transcript, lịch sử Task hoặc dữ liệu phía provider. Tính năng tự học/ghi nhớ suy diễn không được bật ngầm. Xem [ADR 0135](docs/adr/0135-memory-core-runtime.md).

## LLM Gateway

Trong Studio, vào **LLM Gateway** để kết nối tài khoản, quản lý pool và model. Provider dùng adapter `llm_gateway` với `gateway_binding = "account:<account_key>"` hoặc `"pool:<vendor>"`; các cấu hình OpenAI có gateway binding cũng đi qua gateway nội bộ. Token tài khoản được mã hóa, nên cần chuẩn bị khóa credential trước.

Gateway dùng snapshot tài khoản trong RAM, làm mới token nền và failover có giới hạn trước khi stream phát nội dung. Đây là runtime nội bộ cho voice/Jobs, không mở một proxy `/v1` công khai. API quản trị nằm dưới `/api/admin/llm-gateway/*`. Xem [ADR 0130](docs/adr/0130-internal-llm-gateway-oauth-account-pools.md).

## Dữ liệu và sao lưu

| Dữ liệu | Nơi lưu |
| --- | --- |
| Agents, Devices, Providers, reminders, gateway accounts, transcript | `[database].url`, mẫu: `data/voice-agent.db` |
| TaskManager và khóa sở hữu process | `[orchestrator].tasks_store_dir`, gồm `tasks.db`, `tasks.owner.lock` |
| Agent state/cấu hình, prompts, Skills, báo cáo và work files | Các đường dẫn `store_dir`, `prompts_dir`, `skills_dir`, `reports_dir`, `work_dir` trong `[orchestrator]` |
| Wiki Markdown và index dẫn xuất | `[wiki].vault_dir` |
| Memory Fact, summary và forget tombstones | `[memory].vault_dir` |
| Model assets | `models/` |
| Khóa mã hóa credential | File khóa riêng hoặc biến môi trường, nằm ngoài database |

System Backup hỗ trợ export bundle JSON/snapshot SQLite, inspect và import online qua `/api/admin/system/backup/*`. Đây là backup của control-plane database, không phải bản sao toàn bộ deployment: sao lưu riêng Task store, Wiki/Memory Vault, báo cáo, Skills/prompts và khóa credential. Memory Vault phải gồm cả khóa HMAC tombstone trong `_deployment`.

Không ghép `tasks.db` cũ với control-plane được import sang deployment mới: import có thể ánh xạ lại Device row ID, làm task cũ trỏ nhầm Device. Xem [ADR 0131](docs/adr/0131-database-backup-export-and-transactional-import.md) và [giới hạn Memory](docs/adr/0135-memory-core-runtime.md).

## Kiểm thử và kiểm tra mã nguồn

**Mọi test/check đều chạy qua `scripts/agent-test.sh`**, từ thư mục gốc. Wrapper giới hạn output, giữ exit code và lưu log đầy đủ khi lỗi. Chạy test có phạm vi hẹp trước khi mở rộng:

```bash
# Thay reminder bằng tên/filter phù hợp với thay đổi.
scripts/agent-test.sh targeted reminder
scripts/agent-test.sh check
scripts/agent-test.sh package

# Kiểm tra format và lint.
scripts/agent-test.sh run -- cargo fmt --all -- --check
scripts/agent-test.sh run -- cargo clippy --workspace --all-targets --all-features -- -D warnings

# Sau các kiểm tra có phạm vi hẹp.
scripts/agent-test.sh full
scripts/agent-test.sh run -- cargo test --locked --workspace --features qualification-providers

# Admin Web: cài dependencies bằng npm ci trước.
scripts/agent-test.sh web test
scripts/agent-test.sh web typecheck
scripts/agent-test.sh web build
```

`web build` bao gồm typecheck. Các thư viện `vendor/` bị loại khỏi root workspace, nên `full` không chạy bộ test riêng của chúng; khi sửa vendor, dùng wrapper `run --` với `cargo test --manifest-path vendor/<repo>/Cargo.toml` và filter phù hợp.

Các gate cần model/provider thật là opt-in; kết quả deterministic không chứng minh hardware, API thật hoặc Docker đã hoạt động. Xem [chiến lược test](docs/testing/00-test-strategy.md).

## Docker

Repository có Compose production và qualification/test; nginx phục vụ Studio và proxy API/WS đến backend. Dùng wrapper từ thư mục gốc:

```bash
bash scripts/compose.sh --help
scripts/agent-test.sh run -- bash scripts/compose.sh prod config
bash scripts/compose.sh prod build
./scripts/compose.sh build server web  # Build selected services (defaults to prod)
./scripts/compose.sh build all         # Build all images; also: ./scripts/compose.sh build
bash scripts/compose.sh prod up
./scripts/compose.sh prod down web         # Stop and remove web only; keep data
./scripts/compose.sh prod down server web  # Stop and remove selected services
./scripts/compose.sh prod down all         # Remove the whole stack; keep data
```

Trước khi chạy production, tạo `docker/config.prod.toml` và `docker/secrets.env` từ [config mẫu](docker/config.prod.example.toml) và [env mẫu](docker/secrets.env.example), chỉnh provider credentials, bind và runtime library. Chuẩn bị thư mục data/models và quyền ghi theo [`compose.prod.yaml`](compose.prod.yaml); web mặc định ở `http://127.0.0.1:8080`.

Compose production mount chỉ đọc `docker/credential-keys.json` và `runtime/kokoro-vi`; cần chuẩn bị các đường dẫn này trước khi chạy. Kokoro cần Python env tương thích Linux trong container. Nếu cổng 8080 đã được dùng, chạy `WEB_PORT=8082 bash scripts/compose.sh prod up` và dùng cùng `WEB_PORT` cho các lần `up` tiếp theo.

Để lưu cổng cho các lần chạy sau, đặt `WEB_PORT=8082` trong file `.env` ở thư mục gốc (không commit), rồi chạy `bash scripts/compose.sh prod up`. Studio sẽ ở `http://127.0.0.1:8082`. Khi đổi địa chỉ cho thiết bị truy cập, cập nhật `server.public_ws_url` trong `docker/config.prod.toml` cho khớp địa chỉ web được publish.

Compose mặc định lấy data/models production tại `/mnt/storage/ai-agent-voice/docker/prod/`; có thể đổi bằng `VOICE_PROD_DATA_DIR` và `VOICE_PROD_MODELS_DIR`. [Storage checker](scripts/check-docker-storage.py) kiểm tra đường dẫn Cargo build nếu có `.cargo/config.toml` và thư mục storage tương ứng. Docker/containerd dùng storage mặc định; image, volume và cache của Docker build nằm trong storage của Docker, kể cả cache mount có đường dẫn `/mnt/storage` bên trong container. Khi dùng máy khác, chuẩn bị volume paths có quyền ghi và đặt `APP_UID`/`APP_GID` phù hợp. Backend production không publish cổng trực tiếp ra host.

Chạy kiểm thử Docker qua wrapper để giới hạn log:

```bash
scripts/agent-test.sh run -- bash scripts/compose.sh test run
```

Lệnh này chạy checks và smoke tests trong môi trường test riêng, **xóa test data** khi teardown. Không dùng dữ liệu production cho test stack.

## Provider benchmarks

`provider-bench-av` đo boundary ASR/VAD/LLM, dùng workload đã version control; warmup không tính vào số liệu. LLM đo TTFT tại `TextDelta` không rỗng đầu tiên, total latency và chunk/tool-call counts.

```bash
scripts/agent-test.sh run -- cargo run --locked --release -p voice-agent-server --bin provider-bench-av -- \
  llm --config config.toml \
  --workload benchmarks/performance_tester/workloads/llm-v1.json \
  --warmup 1 --iterations 5 --output target/benchmarks/llm.json

scripts/agent-test.sh run -- cargo run --locked --release -p voice-agent-server --bin provider-bench-av -- \
  asr --config config.toml \
  --workload benchmarks/performance_tester/workloads/asr-v1.json \
  --feed burst --warmup 1 --iterations 5

scripts/agent-test.sh run -- cargo run --locked --release -p voice-agent-server --bin provider-bench-av -- \
  vad --config config.toml \
  --workload benchmarks/performance_tester/workloads/vad-v1.json \
  --warmup 1 --iterations 5
```

TTS dùng `provider-bench`, với mode `provider` đo tới PCM hoặc `delivery` đo tới Opus packet sẵn sàng gửi:

```bash
scripts/agent-test.sh run -- cargo run --locked --release -p voice-agent-server --bin provider-bench -- \
  tts provider --config config.toml --warmup-runs 1 --runs 5
```

Các benchmark dùng provider được chọn trong cấu hình và có thể tải model/gọi dịch vụ thật. Report không ghi API key; thời gian phụ thuộc host, model và execution settings.

## Thứ tự đọc tài liệu

1. [Domain model](CONTEXT.md) và [thuật ngữ](GLOSSARY.md).
2. [Các flow voice pipeline](docs/flows/README.md), [Studio Assistant](docs/api/studio-assistant.md) và [System MCP](docs/api/system-mcp.md).
3. [Cấu hình/binding provider](docs/api/provider-config-bindings.md), [Provider/MCP testing](docs/api/live-testing.md), [tool allowlist](docs/api/tool-allowlist.md) và [Postman collection](docs/api/00-all-apis.postman_collection.json).
4. [Kiến trúc Task/Wiki hiện tại — ADR 0127](docs/adr/0127-orchestrator-flow-task-and-wiki-runtime.md), [LLM Gateway — ADR 0130](docs/adr/0130-internal-llm-gateway-oauth-account-pools.md) và [Memory — ADR 0135](docs/adr/0135-memory-core-runtime.md).
5. [Chiến lược và contract tests](docs/testing/README.md), [quyết định kiến trúc](docs/adr/).

Một số tài liệu implementation/flow cũ mô tả plugin runtime hoặc Job engine đã được thay thế. Khi đối chiếu, xem trạng thái Accepted/Superseded và ADR mới liên quan, đặc biệt ADR 0127.
