# voice-agent-server — Development Documentation

Bộ tài liệu kiến trúc và phát triển cho một voice-agent server Rust tối giản với WebSocket protocol v1.

## Mục tiêu

Dự án không port bất kỳ reference implementation nào theo kiểu 1:1. Thay vào đó, dự án định nghĩa **wire protocol + behavior cốt lõi** của riêng mình và xây kiến trúc Rust nhỏ, rõ ràng, dễ kiểm thử và dễ thay provider.

Core V1 phục vụ mọi **Voice Protocol Client** tuân thủ contract; firmware reference chỉ là nguồn đối chiếu đã pin, không phải loại client duy nhất.

Core V1:

- OTA/discovery endpoint.
- Required SQLite control plane and database-backed Device admission for every WS connection.
- WebSocket protocol v1.
- Opus uplink/downlink.
- VAD + manual listen mode.
- ASR provider.
- Streaming LLM provider.
- TTS provider + Opus pacing.
- Short-term dialogue.
- Abort/barge-in/cancellation.
- Device MCP (`initialize`, `tools/list`, `tools/call`).

Không nằm trong Core V1 như một dependency runtime bắt buộc: manager web/mobile, multi-user, MQTT/UDP gateway, RAG, voiceprint, billing/quota, plugin hot-load, long-term memory. Admin Web tại `apps/admin-web/` là công cụ quản trị tùy chọn, giao tiếp với server chỉ qua Admin API công khai và không sở hữu Voice Session state.

## Chạy nhanh Protocol V1

Khởi động server với cấu hình mẫu:

```bash
VOICE_AGENT_CONFIG=config.example.toml cargo run -p voice-agent-server --bin voice-agent-server
VOICE_AGENT_CONFIG=config.toml cargo run -p voice-agent-server --bin voice-agent-server
```

Database và Device admission luôn bật. Provision Agent và Device `reference-client-01`
qua Admin API trước khi chạy client, hoặc bật enrollment và claim mã từ thiết bị.
Admin API vẫn cần `api.enabled=true` và admin token riêng. Không có Device thì OTA/WS
trả 403 khi enrollment tắt. Xem [flow database và enrollment](docs/flows/08-database-device-enrollment.md)
để chuyển cấu hình cũ và chuẩn bị dữ liệu.

Server tự tạo thư mục cha của SQLite (mặc định `data/`) trước khi mở database và
chạy migration. Server không chạm vào model ở bước này: mỗi local provider tự tải
model của nó vào `models/<KIND>/<provider>` khi Provider Runtime Manager materialize nó.
File đã tồn tại và khác 0 byte thì dùng ngay, không gọi mạng và không kiểm tra checksum.
Vì vậy server khởi động được với model directory rỗng hoặc chưa có, và thời gian tải
nằm ngoài `provider_runtime.startup_timeout_ms`. Provider có `preload = true` được tải
sớm; provider còn lại chỉ tải khi Template hoặc Session thực sự dùng nó. ZeroTTS tải
**toàn bộ** voice nó hỗ trợ, không chỉ voice đang cấu hình.

## Admin Web tùy chọn

Vue Admin nằm trong workspace tại [`apps/admin-web/`](apps/admin-web/). Khởi động
server với `api.enabled=true`, sau đó ở terminal khác chạy:

```bash
cd apps/admin-web
cp .env.example .env
npm install
npm run dev
```

Mở `http://127.0.0.1:5173`, rồi nhập admin bearer token vào form kết nối. Vite
proxy các request `/api/admin/*` đến server tại `http://127.0.0.1:8000` theo mặc
định; xem [hướng dẫn đầy đủ của Admin Web](apps/admin-web/README.md).

Downloader dùng buffer 64 KiB, timeout kết nối 15 giây, timeout 15 phút cho mỗi
lần tải và tối đa 3 lần thử cho lỗi mạng/HTTP tạm thời. Mỗi file được tải vào
`<target>.part` rồi mới atomic rename, nên một lần tải dở không bao giờ để lại file trông
như đã sẵn sàng. ONNX Runtime và Kokoro G2P vẫn cần cài riêng. Voicepack Kokoro được
tải và chuyển đổi tự động bởi provider; xem [hướng dẫn Kokoro](docs/kokoro-vi-provider.md).

Chạy các gate tự động hiện có:

```bash
./scripts/test-all.sh
```

## Provider benchmarks

`provider-bench-av` đo trực tiếp boundary provider; warmup không đi vào số liệu đo. Workload
được version control trong `benchmarks/performance_tester/workloads/`. Report JSON bao gồm raw
samples và summary min/mean/p50/p95/p99; không ghi API key.

LLM provider benchmark đo TTFT tại `TextDelta` không rỗng đầu tiên (không tính empty delta hay
tool call), cùng total latency, số text chunk, số ký tự và tool-call count:

```bash
cargo run --release -p voice-agent-server --bin provider-bench-av -- \
  llm --workload benchmarks/performance_tester/workloads/llm-v1.json \
  --warmup 1 --iterations 5 --output target/benchmarks/llm.json
```

ASR và VAD dùng cùng binary với workload fixture canonical:

```bash
cargo run --release -p voice-agent-server --bin provider-bench-av -- \
  asr --workload benchmarks/performance_tester/workloads/asr-v1.json \
  --feed burst --warmup 1 --iterations 5

cargo run --release -p voice-agent-server --bin provider-bench-av -- \
  vad --workload benchmarks/performance_tester/workloads/vad-v1.json \
  --warmup 1 --iterations 5
```

## Thứ tự đọc

1. [`docs/00-overview.md`](docs/00-overview.md)
2. [`docs/01-system-architecture.md`](docs/01-system-architecture.md)
3. [`docs/02-codebase.md`](docs/02-codebase.md)
4. [`docs/03-module-contracts.md`](docs/03-module-contracts.md)
5. [`docs/04-configuration.md`](docs/04-configuration.md)
6. [`docs/05-development-workflow.md`](docs/05-development-workflow.md)
7. Các flow trong [`docs/flows/`](docs/flows/)
8. Chiến lược test trong [`docs/testing/`](docs/testing/)
9. ADR trong [`docs/adr/`](docs/adr/)

## Nguồn tham chiếu external

- Server: `xinnan-tech/xiaozhi-esp32-server`
- Firmware: `78/xiaozhi-esp32`

Chi tiết mapping sang code Rust đề xuất nằm tại [`docs/reference/source-map.md`](docs/reference/source-map.md).
# Device enrollment over WebSocket

Unknown devices can connect to a separate enrollment WS, display and hear their
six-digit code, then be claimed through the existing Admin API. This connection
has no conversational providers or transcript. Prepare Vietnamese WAV assets
before enabling `database.devices.enrollment` with `transport="websocket"`:

```bash
python3 scripts/prepare-enrollment-assets.py
```

See [setup and wire flow](docs/device-enrollment-websocket.md). Explicit
`transport="ota"` preserves the previous activation/polling flow.
