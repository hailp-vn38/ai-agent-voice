# 02 — Cấu trúc codebase hiện tại

`voice-agent-server` là crate Rust production duy nhất của workspace. Mã production Rust nằm trong `src/`; test integration, fixture và helper test không được trở thành public module của server. Vue Admin là application độc lập tại `apps/admin-web/`, giao tiếp với server qua Admin API công khai.

```text
apps/
└── admin-web/                   # Vue Admin UI, npm/Vite toolchain và test frontend
crates/voice-agent-server/
├── migrations/                  # SQLite schema migrations
├── src/
│   ├── main.rs                  # bootstrap binary production
│   ├── lib.rs                   # public module surface của crate
│   ├── app/                     # HTTP/Admin/OTA/WebSocket/enrollment routes
│   ├── audio/                   # Opus, resample, pacing, canonical audio, VAD segmenter
│   ├── benchmark/               # workload, runner, stats và report benchmark
│   ├── bin/                     # developer binaries chạy cùng server package
│   ├── config/                  # typed config, defaults và validation
│   ├── database/                # SQLite persistence và policy truy cập
│   ├── models/                  # manifest, acquisition, verification, startup preparation
│   ├── protocol/                # Voice Protocol V1 wire contract
│   ├── providers/               # adapter/factory/catalog/runtime snapshot AI
│   ├── services/                # orchestration của provider runtime và enrollment
│   ├── session/                 # Voice Session, actor, turn, prompt, delivery
│   ├── tools/                   # Device MCP, external MCP và tool round
│   ├── workers/                 # bounded runtime/lease cho VAD, ASR, LLM, TTS, Vision
│   ├── lifecycle.rs             # application shutdown/drain lifecycle
│   ├── startup_handshake.rs     # child-process startup artifact
│   └── telemetry.rs             # privacy-safe telemetry
└── tests/
    ├── fixtures/                # deterministic Opus/JSON fixtures nhỏ
    ├── support/                 # setup/router helpers chỉ cho integration test
    └── *.rs                     # test theo public seam hoặc module contract
```

## Ownership

### Production modules

- `apps/admin-web/` sở hữu UI Admin và state phía trình duyệt; không import mã Rust nội bộ mà chỉ gọi Admin API/contract công khai.
- `app/` chỉ nhận HTTP/WebSocket và map chúng vào application state/session events; không quyết định dialogue policy.
- `session/` sở hữu Voice Session state, turn ownership, writer ordering và orchestration từ ingress tới terminal outcome.
- `providers/` định nghĩa seam adapter cho AI runtime; adapter không biết Voice Session hoặc WebSocket.
- `workers/` sở hữu bounded worker lifecycle, lease và acknowledgement; worker không sở hữu Voice Session state.
- `services/` phối hợp configuration/runtime/database lifecycle mà không lộ chi tiết đó cho route handlers.
- `tools/` owns Device MCP và External MCP execution; SessionActor quyết định khi nào tool result được đưa vào một LLM round.
- `audio/` chỉ xử lý format, codec, segmentation và pacing; không giữ WebSocket sender.

### Binaries developer

`src/bin/` chỉ chứa công cụ dùng lại implementation server: benchmark provider, preflight offline và core check. Không đặt WebSocket client, Admin CLI, mock MCP server hoặc harness qualification vào đây. Nếu một công cụ sau này cần được chạy độc lập như external client, nó phải là project/tool độc lập và đi qua HTTP/WS public seam.

### Tests và fixtures

- Integration test gọi router/HTTP/WebSocket công khai và dùng `tests/support/` cho setup chung.
- `tests/fixtures/` thuộc test/preflight của server. Fixture Phase 5 Opus hiện thuộc server vì preflight là owner của contract đó.
- Chỉ tạo `tests/support/voice_ws_client.rs` khi ít nhất hai test cần cùng một flow protocol. Một helper chưa có caller thứ hai là seam giả và không nên tạo trước.
- Test Vision nằm trong `tests/vision_api.rs`; tên file mô tả interface HTTP đang xác minh, không mô tả một client implementation đã bị xoá.

## Quy tắc dependency

Không được tạo dependency kiểu:

```text
ASR provider -> SessionActor
TTS provider -> WebSocket
LLM provider -> MCP transport trực tiếp
VAD -> Dialogue History
SpeechOutput -> WebSocket
worker -> Voice Session state
```

Mọi output quay về `SessionActor` qua event/acknowledgement. Đây là seam public/internal rõ ràng: caller và test công khai quan sát protocol/HTTP; implementation bên trong có thể tiếp tục tách nhỏ mà không đổi contract.

## Hướng refactor

Không tách file chỉ vì nhiều dòng. Tách khi một module đang mang hai responsibility độc lập và có interface nhỏ hơn, sâu hơn sau khi tách. Các điểm cần đánh giá ở ticket riêng là `app/state.rs`, `workers/tts.rs`, `config/mod.rs`, `services/provider_runtime/registry.rs` và `session/profile.rs`.
