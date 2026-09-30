# Rust Reference Client — Database Provisioning & End-to-End Integration Guide

## 1. Mục tiêu

Sau khi server bổ sung SQLite và Admin API, `voice-reference-client` cần được mở rộng thành **reference integration client** kiểm thử đồng thời:

- control plane: Agent, Template, Provider, Device, External MCP Server và các binding;
- optimistic concurrency: `revision` / `If-Match`;
- provider runtime state và Provider Test API;
- DB-backed Voice admission;
- Agent → Template → Provider resolution;
- External MCP discovery;
- Voice WebSocket turn hoàn chỉnh.

Completion flow mục tiêu:

```text
Fresh SQLite
    ↓
Admin provisioning
    ↓
restart server
    ↓
runtime verification
    ↓
Device admission
    ↓
Agent
    ↓
Default Template
    ↓
Provider bindings
    ↓
External MCP discovery
    ↓
Voice turn
    ↓
TTS complete
```

Client này không thay thế server unit test. Nó kiểm tra public contract giữa Admin API, SQLite, RuntimeCatalog và Voice Protocol.

Đây là **Reference Integration Client phục vụ qualification**, không phải Admin CLI vận hành đầy đủ:

- `AdminClient` là typed reusable library;
- resource command chỉ đủ để scenario và người chạy qualification quan sát/debug;
- không cam kết bao phủ mọi workflow vận hành hoặc trở thành management console tổng quát.

Effort được quản lý riêng tại:

```text
.scratch/rust-reference-client-database-integration/
```

với các phase/ticket RC-DB1 đến RC-DB6. Ticket database integration qualification hiện có chỉ orchestration, consume artifact và chạy gate cuối; dependency graph của ticket đó chỉ được đổi sau khi design này hoàn tất.

### Hai tầng bằng chứng

Qualification bắt buộc phải deterministic và chạy ổn định trong CI:

```text
Mandatory Provider Qualification
    → public Provider Test API
    → deterministic provider doubles/local fixtures
    → loaded-runtime semantics
    → không cần external credential/model
```

Bằng chứng môi trường thật được report riêng:

```text
Optional Runtime Evidence
    → real ASR model
    → real LLM remote API
    → real TTS model/provider
    → real External MCP
    → hardware/HIL
    → PASS | FAIL | NOT_RUN
```

`FAIL` hoặc `NOT_RUN` ở Optional Runtime Evidence không thay đổi kết quả Mandatory Qualification.

### Qualification build của production binary

Mandatory Qualification build crate server với feature compile-time explicit:

```toml
[features]
default = []
qualification-providers = []
```

Binary vẫn là `voice-agent-server` và chạy production `main`, config loading, database startup,
migrations, Provider Load Plan, RuntimeCatalog và listener path. Feature chỉ thêm vào compile-time
registry các adapter `qualification_vad`, `qualification_asr`, `qualification_llm` và
`qualification_tts`; không có runtime switch hoặc shortcut inject `ProviderSet`/`AppState`.

Qualification adapters đi qua cùng pipeline:

```text
typed adapter config
→ ProviderConfigValidator
→ DB desired provider
→ Template binding
→ restart
→ ProviderLoadPlan
→ RuntimeCatalog
→ Provider Test API
→ Voice Session
```

Chúng deterministic tuyệt đối: VAD trả probabilities/boundaries cố định, ASR trả transcript local
cố định, LLM trả response/tool behavior cố định, TTS trả valid deterministic PCM/audio. Chúng không
dùng network, SecretResolver, `secret_ref`, model download, environment credential hoặc randomness.

Default/release build không chứa hoặc advertise adapter qualification. CI có hai gates: default
build chứng minh `qualification_*` absent; qualification build chứng minh chúng present. Đây là
qualification build của production binary, không phải khẳng định bit-for-bit giống deployed release
artifact.

---

## 2. Constraint quan trọng: Provider mới cần restart

Theo contract server:

```text
Admin API create/update Provider
        ↓
SQLite desired state
        ↓
RuntimeCatalog hiện tại KHÔNG thay đổi
```

Provider runtime được build khi startup:

```text
startup
  ↓
ProviderLoadPlan
  ↓
SecretResolver
  ↓
load runtime/model
  ↓
RuntimeCatalog
```

Vì vậy integration flow **không được** giả định:

```text
create Provider
→ bind Template
→ Voice dùng ngay Provider mới
```

Flow chuẩn:

```text
provision
→ restart
→ verify
→ voice smoke
```

Rust client phải model restart barrier này rõ ràng.

---

## 3. Không làm phình binary Voice hiện tại

Giữ binary hiện tại:

```text
voice-reference-client
```

cho Voice Protocol compatibility.

Bổ sung binary riêng:

```text
voice-admin-client
```

Ví dụ file:

```text
crates/voice-reference-client/src/bin/voice-admin-client.rs
```

Có thể dùng `[[bin]]` explicit nếu layout source khác, nhưng artifact bắt buộc mang đúng tên `voice-admin-client`.

---

## 4. Cấu trúc module đề xuất

```text
crates/voice-reference-client/
├── src/
│   ├── lib.rs
│   ├── main.rs
│   ├── bin/
│   │   └── voice-admin-client.rs
│   ├── admin/
│   │   ├── mod.rs
│   │   ├── client.rs
│   │   ├── auth.rs
│   │   ├── error.rs
│   │   ├── models.rs
│   │   ├── agents.rs
│   │   ├── templates.rs
│   │   ├── providers.rs
│   │   ├── devices.rs
│   │   └── mcp.rs
│   ├── scenario/
│   │   ├── mod.rs
│   │   ├── spec.rs
│   │   ├── state.rs
│   │   ├── provision.rs
│   │   ├── verify.rs
│   │   ├── smoke.rs
│   │   └── disable.rs
│   └── mock_mcp/
│       ├── mod.rs
│       ├── server.rs
│       └── tools.rs
└── tests/
    ├── fixtures/
    └── scenarios/
```

---

## 5. AdminClient

Tạo một abstraction duy nhất:

```rust
pub struct AdminClient {
    base_url: url::Url,
    http: reqwest::Client,
    auth: AdminCredential,
}
```

Không để scenario gọi `reqwest` rải rác.

Các method chính:

```rust
impl AdminClient {
    async fn create_provider(
        &self,
        req: CreateProviderRequest,
    ) -> Result<ProviderView, AdminError>;

    async fn get_provider(
        &self,
        key: &ResourceKey,
    ) -> Result<ProviderView, AdminError>;

    async fn create_template(
        &self,
        req: CreateTemplateRequest,
    ) -> Result<TemplateView, AdminError>;

    async fn bind_template_provider(
        &self,
        template_key: &ResourceKey,
        expected_revision: u64,
        req: BindTemplateProviderRequest,
    ) -> Result<TemplateView, AdminError>;

    async fn create_agent(
        &self,
        req: CreateAgentRequest,
    ) -> Result<AgentView, AdminError>;

    async fn assign_template(
        &self,
        agent_key: &ResourceKey,
        expected_revision: u64,
        req: AssignTemplateRequest,
    ) -> Result<AgentView, AdminError>;

    async fn create_mcp_server(
        &self,
        req: CreateMcpServerRequest,
    ) -> Result<McpServerView, AdminError>;

    async fn bind_agent_mcp(
        &self,
        agent_key: &ResourceKey,
        expected_revision: u64,
        req: BindAgentMcpRequest,
    ) -> Result<AgentView, AdminError>;

    async fn create_device(
        &self,
        req: CreateDeviceRequest,
    ) -> Result<DeviceView, AdminError>;
}
```

---

## 6. Authentication

Admin API dùng Bearer token riêng.

Không khuyến nghị truyền token qua CLI argument vì dễ xuất hiện trong shell history/process list.

```bash
export VOICE_ADMIN_TOKEN='...'

voice-admin-client   --admin-url http://127.0.0.1:8000/api/admin/   --admin-token-env VOICE_ADMIN_TOKEN   ...
```

Client không log token.

`--admin-url` là exact Admin API base và phải có canonical trailing slash. Chỉ nhận `http`/`https`,
cấm userinfo, query và fragment; `http` chỉ hợp lệ với loopback, còn non-loopback bắt buộc
`https`. Resource endpoint chỉ được join bằng relative path sau validation. Admin `reqwest::Client`
disable redirect và chỉ attach Bearer vào origin đã validate. Harness dựng base từ bound address +
fixed `/api/admin/`; `--ota-url` là contract riêng và không dùng generic Admin URL join.

---

## 7. Resource graph phải test

Không chỉ test Agent/Device/Provider/MCP. **Template bắt buộc phải tham gia**, vì Provider được bind qua Template.

```text
Device
  ↓
Agent
  ├────────────→ External MCP
  │
  ↓
Default Template
  ↓
Provider bindings
  ├── VAD
  ├── ASR
  ├── LLM
  └── TTS
```

Nếu bỏ Template, test có thể chỉ đang dùng server defaults thay vì DB-backed provider binding.

---

## 8. Typed DTOs

Không dùng `serde_json::Value` cho CRUD chính.

```rust
#[derive(Debug, Serialize)]
pub struct CreateAgentRequest {
    pub key: String,
    pub name: String,
    pub enabled: bool,
}

#[derive(Debug, Deserialize)]
pub struct AgentView {
    pub id: i64,
    pub key: String,
    pub name: String,
    pub enabled: bool,
    pub revision: u64,
}
```

Làm tương tự cho:

- Provider
- Template
- Device
- MCP Server
- bindings

Typed DTO giúp phát hiện API drift sớm.

Các wire DTO này được định nghĩa độc lập trong `voice_reference_client::admin::models`. Client không import server domain type, repository row hoặc handler request/response struct.

```text
Server implementation types
    ≠
Reference client wire types
```

Nếu server thay đổi public payload không tương thích, client phải fail compile/test hoặc deserialize thay vì tự động thích nghi qua shared implementation type.

---

## 9. Revision / If-Match

Binding mutation phải theo optimistic concurrency.

Ví dụ Template:

```text
create Template
→ revision=1

bind ASR
→ If-Match: 1
→ revision=2

bind LLM
→ If-Match: 2
→ revision=3

bind TTS
→ If-Match: 3
→ revision=4
```

Agent:

```text
create Agent
→ revision=1

assign Template
→ revision=2

bind MCP
→ revision=3
```

Client phải lấy revision mới nhất từ response, không hard-code revision ban đầu.

Có thể dùng:

```rust
pub struct Versioned<T> {
    pub value: T,
    pub revision: u64,
}
```

---

## 10. ScenarioState

Provisioning phải ghi state ra file để verify sau restart.

```json
{
  "schema_version": 1,
  "run_id": "dbtest_20260929",
  "created_at_unix_ms": 1790654400000,
  "scenario_spec_sha256": "0123456789abcdef...",
  "agent": {
    "key": "dbtest_agent",
    "revision": 3,
    "diagnostic_id": 12
  },
  "template": {
    "key": "dbtest_default",
    "revision": 5,
    "diagnostic_id": 20
  },
  "device": {
    "device_id": "rust-reference-db-test-01",
    "revision": 1,
    "diagnostic_id": 31
  },
  "providers": {
    "asr": {
      "key": "dbtest_asr",
      "revision": 1,
      "diagnostic_id": 40
    },
    "llm": {
      "key": "dbtest_llm",
      "revision": 1,
      "diagnostic_id": 41
    },
    "tts": {
      "key": "dbtest_tts",
      "revision": 1,
      "diagnostic_id": 42
    }
  },
  "mcp_server": {
    "key": "dbtest_mcp",
    "revision": 1,
    "diagnostic_id": 50
  }
}
```

Identity invariant:

```text
Public resource identity
    = immutable key/device_id

Database primary key
    = implementation detail
```

`diagnostic_id` là metadata optional nếu public response có trả về. Client không dùng nó để dựng URL, lookup, bind, resume hoặc cleanup. Mọi mutation tiếp theo dùng public identity cùng `revision` mới nhất nhận từ response.

`ScenarioState` là immutable handoff artifact giữa hai process lifetime, không phải journal để
resume. Nó chứa schema version, `run_id`, creation time, raw-spec digest, public resource
key/device_id, revision và runtime observation cần verify; không chứa endpoint URL, admin token, secret
value, `secret_ref`, authorization header, provider prompt/result/audio. Writer phải create-new,
validate nội dung đầy đủ, ghi temporary sibling, flush rồi atomically rename mà không overwrite
state có sẵn; trên Unix file dùng mode `0600` khi có thể. File tồn tại, malformed, unsupported
schema version hoặc digest không match là failure trước bất kỳ HTTP/Voice side effect
nào của `verify`, `voice-smoke` hoặc `disable`.

Identity layers không được nhập làm một:

```text
Scenario identity
    = run_id + scenario_spec_sha256 + resource keys/device_id

Process identity
    = harness-owned child + startup_nonce + handshake artifact

Network endpoint
    = ephemeral observation của từng process lifetime
```

PID chỉ là diagnostic/cross-check; không phải process identity độc lập. Old admin URL, OTA URL hoặc
bound address không chứng minh process sau restart là cùng deployment. Automated harness lấy
endpoint hiện tại từ handshake của từng child; manual command luôn nhận endpoint tại invocation đó.

`scenario_spec_sha256` là lowercase 64-character SHA-256 của raw TOML bytes trước parse. Mọi command
sau provisioning phải đọc lại cùng spec, kiểm tra digest trước network side effect và không
canonicalize TOML hay nhúng bản sao đầy đủ của spec vào state.

CLI:

```bash
voice-admin-client scenario provision   --spec tests/scenarios/db-flow.toml   --state .tmp/db-flow-state.json
```

Sau restart:

```bash
voice-admin-client scenario verify   --spec tests/scenarios/db-flow.toml   --state .tmp/db-flow-state.json
voice-admin-client scenario voice-smoke   --spec tests/scenarios/db-flow.toml   --state .tmp/db-flow-state.json   --ota-url <current-url>
voice-admin-client scenario disable   --spec tests/scenarios/db-flow.toml   --state .tmp/db-flow-state.json   --admin-url <current-url>
```

---

## 11. Declarative ScenarioSpec

Không bắt user chạy hàng chục command thủ công.

```toml
[scenario]
key_prefix = "rust"

[agent]
key_suffix = "agent"
name = "Rust Integration Agent"
enabled = true

[template]
key_suffix = "template"
name = "Rust Integration Template"
language = "vi-VN"
prompt = "Bạn là trợ lý dùng cho integration test."
enabled = true

[providers.asr]
key_suffix = "asr"
adapter = "qualification_asr"
enabled = true

[providers.asr.config]
transcript = "xin chao"

[providers.llm]
key_suffix = "llm"
adapter = "qualification_llm"
enabled = true

[providers.llm.config]
response = "Xin chào từ qualification provider."

[providers.tts]
key_suffix = "tts"
adapter = "qualification_tts"
enabled = true

[providers.tts.config]
sample_rate_hz = 24000

[device]
id_suffix = "device"

[mcp]
key_suffix = "mcp"
url = "http://127.0.0.1:9901/mcp"
enabled = true
```

Mandatory scenario cũng bind `[providers.vad]` với `qualification_vad`; snippet rút gọn phần config
VAD cho dễ đọc.

Runner generate một lần trước network side effect:

```text
run_token = 24 lowercase random hex từ CSPRNG
run_id    = "it_" + run_token
final key = <key_prefix>_<run_token>_<role_suffix>
```

`--run-id` explicit phải parse đúng canonical `it_` + 24 lowercase hex; runner lấy phần hex làm
`run_token`, không hash/truncate/normalize. Tất cả final resource keys và derived Device ID được
materialize, validate cùng lúc rồi persist vào state trước request đầu tiên. Bất kỳ identity nào
vi phạm server grammar/bound (`Resource Key <=64`, Device ID `<=128`) fail
`scenario_identity_invalid`. Sau request đầu tiên không regenerate, truncate hoặc collision-retry.

---

## 12. Provision order

Scenario runner provision deterministic theo dependency graph:

```text
1. GET /ready
2. GET provider adapter descriptors

3. create VAD Provider nếu cần
4. create ASR Provider
5. create LLM Provider
6. create TTS Provider

7. create Template

8. bind Template → VAD
9. bind Template → ASR
10. bind Template → LLM
11. bind Template → TTS

12. create Agent

13. assign Agent → Template
14. set default Template assignment

15. create MCP Server
16. bind Agent → MCP

17. create Device
18. bind Device → Agent

19. GET lại resources
20. ghi ScenarioState
21. report RESTART_REQUIRED
```

---

## 13. Provision command

```bash
voice-admin-client scenario provision   --admin-url http://127.0.0.1:8000   --admin-token-env VOICE_ADMIN_TOKEN   --spec tests/scenarios/db-flow.toml   --state .tmp/db-flow-state.json
```

Output ví dụ:

```text
READY                  PASS
PROVIDER ASR           CREATED
PROVIDER LLM           CREATED
PROVIDER TTS           CREATED
TEMPLATE               CREATED
TEMPLATE BINDINGS      PASS
AGENT                  CREATED
AGENT TEMPLATE         PASS
MCP SERVER             CREATED
AGENT MCP BINDING      PASS
DEVICE                 CREATED
DEVICE AGENT BINDING   PASS

RESTART_REQUIRED       YES
STATE                  .tmp/db-flow-state.json
```

Provision command không được tự khẳng định runtime mới đã active.

---

## 14. Verify sau restart

Sau restart, verify Provider:

```text
runtime_status = loaded
runtime_matches_desired = true
requires_restart = false
```

Required Provider mà `not_loaded`, `unavailable` hoặc mismatch thì verify fail.

Flow:

```text
ScenarioState
    ↓
GET Provider resources
    ↓
verify DB state
    ↓
verify runtime state
    ↓
Provider diagnostics
```

---

## 15. Provider diagnostic

RC-DB5 phụ thuộc public Provider Descriptor/Test API được định nghĩa tại `docs/provider-descriptor-capability-and-test-api-guide.md`. Reference Integration Client không tạo diagnostic seam thứ hai và không được hạ completion gate xuống chỉ kiểm tra `runtime_status`.

Mandatory Provider Qualification chạy các endpoint public bằng deterministic provider doubles/local fixtures. Các ví dụ ASR/LLM/TTS dưới đây mô tả contract cần kiểm tra, không yêu cầu credential, remote service hoặc model thật.

### 15.1 ASR

```text
fixture WAV
→ POST ASR test
→ transcript non-empty
```

Verify:

```text
status=success
elapsed_ms > 0
audio_duration_ms > 0
rtf >= 0
```

Không cần assert transcript exact nếu model không hoàn toàn deterministic.

### 15.2 LLM

Input:

```text
"Trả lời đúng một câu ngắn."
```

Verify:

```text
status=success
text non-empty
```

Provider diagnostic không tool-call.

### 15.3 TTS

Input:

```text
"Xin chào, đây là kiểm thử TTS."
```

Verify:

```text
HTTP 200
Content-Type audio/wav
WAV hợp lệ
sample count > 0
duration > 0
```

Real ASR/LLM/TTS chạy lại cùng public contract chỉ là Optional Runtime Evidence. Kết quả phải report `PASS | FAIL | NOT_RUN` riêng và không được gộp vào mandatory result.

---

## 16. Voice smoke test

Sau provider verification, tái sử dụng WS client hiện tại.

```rust
run_text_turn(TextTurnRequest {
    ota_url,
    device_id: state.device.device_id.clone(),
    client_id,
    text,
    config,
}).await?;
```

Flow:

```text
Rust Client
    ↓
Voice WS
    ↓
DB Device lookup
    ↓
Agent
    ↓
Default Template
    ↓
Provider bindings
    ↓
RuntimeCatalog
    ↓
LLM
    ↓
TTS
```

Completion gate:

```text
WS accepted
TTS started
audio packets received
TTS stopped
post-stop quiet period valid
```

---

## 17. Device admission regression

Trước provisioning:

```text
unknown Device-ID
→ 403
```

Sau provisioning:

```text
same Device-ID
→ accepted
```

Đây là gate quan trọng cho DB-backed admission.

---

## 18. Device MCP và External MCP phải tách

Device MCP hiện tại:

```text
self.*
```

External MCP trong DB:

```text
external.<server_key>.<tool>
```

Không dùng chung semantics hoặc CLI flag.

Nên phân biệt:

```text
device-mcp
external-mcp
```

---

## 19. Mock External MCP Server

Integration test không nên phụ thuộc MCP server thật bên ngoài.

Bổ sung mock deterministic:

```bash
voice-admin-client mock-mcp   --listen 127.0.0.1:9901
```

Mock hỗ trợ:

```text
initialize
tools/list
tools/call
```

Tools mẫu:

```text
echo
increment
get_test_value
```

Harness giữ `Arc<MockMcpStats>` và bounded ordered event log trong process; không mount stats
endpoint vào mock. Counter chỉ hỗ trợ summary, còn event log là authority về protocol order:

```rust
pub struct MockMcpStats {
    pub initialize_count: u64,
    pub tools_list_count: u64,
    pub tools_call_count: u64,
}

pub enum MockMcpEventKind {
    Initialize,
    ToolsList,
    ToolsCall { tool: String }, // tên thuộc static tool set của mock
}

pub struct MockMcpEvent {
    pub sequence: u64,
    pub kind: MockMcpEventKind,
}
```

V1 dùng `MAX_MOCK_MCP_EVENTS = 256`; sequence là `u64` bắt đầu từ 1 trong mỗi mock lifecycle.
Storage giữ first-N, không phải ring buffer. Khi event thứ 257 đến, counters tiếp tục tăng, 256 event
đầu giữ nguyên và `overflowed` trở thành sticky `true`. Tool name trong event phải thuộc static
allowlisted mock tools và chịu length bound; arguments/results không được lưu.

Mandatory admission gate assert `overflowed == false`, first `Initialize` tồn tại, first `ToolsList`
tồn tại và Initialize sequence nhỏ hơn ToolsList sequence; overflow fail với stable code
`mock_mcp_event_overflow`. Không dựa timestamp wall-clock. `tools/call` vẫn được chứng minh trong
gate deterministic Tool-round riêng, không suy ra chỉ từ admission stats. Mỗi mock restart tạo
lifecycle/counter/sequence mới; không trộn evidence giữa các run.

---

## 20. MCP test chia 3 lớp

### MCP-1 — provisioning

Verify:

```text
mcp_servers row exists
agent_mcp_bindings exists
```

### MCP-2 — admission/discovery

Mock phải nhận:

```text
initialize
tools/list
```

khi Device mở session.

Đây là deterministic gate cho:

```text
Device
→ Agent
→ MCP binding
→ fresh External MCP discovery
```

### MCP-3 — execution

`tools/call` nên test riêng bằng deterministic LLM/stub hoặc server integration harness.

Không nên làm toàn DB flow phụ thuộc việc model có tự quyết định gọi tool hay không.

---

## 21. Secret handling

Scenario chỉ gửi:

```text
secret_ref
```

Không hỗ trợ plaintext:

```text
api_key
token
password
secret
authorization
```

trong Provider config.

Client có thể reject local sớm, nhưng server vẫn là authority và phải validate lại.

---

## 22. Error model

Không map mọi lỗi thành `anyhow!("request failed")`.

```rust
pub enum AdminError {
    Unauthorized,
    Forbidden,
    NotFound,
    Conflict,
    DatabaseBusy,
    RuntimeUnavailable,
    Validation {
        code: String,
    },
    UnexpectedStatus {
        status: reqwest::StatusCode,
        code: Option<String>,
    },
    Transport(reqwest::Error),
}
```

Điều này cho phép scenario assert lỗi chính xác.

---

## 23. Negative test suite

Tối thiểu:

```text
missing Admin bearer
→ 401

wrong Admin bearer
→ 401

unknown Device
→ 403

disabled Device
→ 403

stale If-Match
→ 409

invalid provider config
→ 400

plaintext api_key trong config_json
→ 400

provider config quá lớn
→ 400

admin raw body quá lớn
→ 413

provider tạo nhưng chưa restart
→ runtime not loaded/mismatch

provider sau restart
→ loaded + matches desired

optional MCP unavailable
→ Voice vẫn được accept

valid MCP
→ initialize + tools/list

valid graph
→ Voice turn complete
```

---

## 24. Stale revision regression

```text
GET Agent
→ revision=3

PATCH Agent If-Match:3
→ success
→ revision=4

PATCH Agent If-Match:3
→ 409
```

Nên có test command riêng hoặc chạy trong scenario test suite.

---

## 25. Provider restart-awareness regression

```text
create Provider
    ↓
GET Provider
    ↓
runtime_matches_desired=false
hoặc runtime_status=not_loaded
    ↓
restart server
    ↓
GET Provider
    ↓
runtime_status=loaded
runtime_matches_desired=true
```

Gate này ngăn implementation vô tình tạo hot-reload trái contract.

---

## 26. Cleanup strategy

### Automated integration

Khuyến nghị dùng:

```text
temporary SQLite DB per test run
```

Flow:

```text
create temp dir
start server với temp DB
run scenario
stop server
remove temp DB
```

### Shared/manual server

Dùng unique prefix:

```text
it_20260929_abc123
```

Cuối flow chỉ soft-disable:

```text
Device
Agent
Template
Provider
MCP Server
```

Không purge dữ liệu production ngoài scope.

---

## 27. CLI đề xuất

### Resource commands

Các command này chỉ là diagnostic surface tối thiểu cho scenario/debug, không phải cam kết một Admin CLI vận hành đầy đủ.

```bash
voice-admin-client provider list
voice-admin-client provider get <key>
voice-admin-client provider create --file provider.json

voice-admin-client template list
voice-admin-client agent list
voice-admin-client device list
voice-admin-client mcp list
```

### Scenario commands

```bash
voice-admin-client scenario provision
voice-admin-client scenario verify
voice-admin-client scenario voice-smoke
voice-admin-client scenario disable
```

### Mock MCP

```bash
voice-admin-client mock-mcp
```

---

## 28. CLI skeleton

```rust
#[derive(clap::Parser)]
struct Args {
    #[arg(long)]
    admin_url: String,

    #[arg(long, default_value = "VOICE_ADMIN_TOKEN")]
    admin_token_env: String,

    #[command(subcommand)]
    command: Command,
}

#[derive(clap::Subcommand)]
enum Command {
    Provider(ProviderCommand),
    Template(TemplateCommand),
    Agent(AgentCommand),
    Device(DeviceCommand),
    Mcp(McpCommand),
    Scenario(ScenarioCommand),
    MockMcp(MockMcpCommand),
}
```

---

## 29. ScenarioRunner

```rust
pub struct ScenarioRunner {
    admin: AdminClient,
}

impl ScenarioRunner {
    pub async fn provision(
        &self,
        spec: ScenarioSpec,
    ) -> Result<ScenarioState, ScenarioError> {
        todo!()
    }

    pub async fn verify(
        &self,
        state: &ScenarioState,
    ) -> Result<VerifyReport, ScenarioError> {
        todo!()
    }
}
```

Report:

```rust
pub struct VerifyReport {
    pub resources_persisted: bool,
    pub providers_loaded: bool,
    pub provider_tests_passed: bool,
    pub mcp_discovered: bool,
    pub voice_turn_completed: bool,
}
```

---

## 30. Automated process lifecycle

`IntegrationHarness` là authority của Mandatory Qualification trong CI:

```text
spawn server
    ↓
wait /ready
    ↓
provision
    ↓
stop server
    ↓
start lại cùng DB
    ↓
wait /ready
    ↓
verify
    ↓
voice-smoke
    ↓
stop server
```

Tuy nhiên:

```text
AdminClient
```

không sở hữu process lifecycle.

Tách:

```text
AdminClient        = HTTP control-plane client
ScenarioRunner     = logical scenario
IntegrationHarness = process lifecycle
```

Ranh giới ownership:

- `AdminClient` chỉ sở hữu HTTP control-plane interaction;
- `ScenarioRunner` chỉ sở hữu logical scenario và state transition;
- `IntegrationHarness` tạo temp directory/SQLite, spawn server và deterministic doubles/mock, chờ readiness, controlled-stop, restart cùng DB, chạy verify/Voice E2E và teardown;
- manual `scenario provision|verify|voice-smoke|disable` dành cho debug hoặc shared server và không đủ để tuyên bố Mandatory Qualification CI đã pass.

### 30.1 Production-process startup và controlled restart

Harness luôn spawn production binary thật. Nó không parse log, reserve port trước hay rebuild
`AppState` để mô phỏng restart. Mỗi spawn tạo startup nonce mới, remove bound-address artifact cũ
và truyền:

```text
VOICE_AGENT_STARTUP_NONCE=<nonce>
VOICE_AGENT_BOUND_ADDRESS_FILE=<temporary path>
```

Hai biến là một atomic configuration contract: cùng absent giữ production startup hiện tại, hoặc
cùng present để bật handshake. Missing counterpart, nonce không phải lowercase canonical UUID,
artifact path không tuyệt đối hoặc parent không tồn tại đều fail trước bind.

Sau bind `127.0.0.1:0`, production server atomically ghi:

```json
{
  "version": 1,
  "startup_nonce": "7df...",
  "pid": 18422,
  "address": "127.0.0.1:54321"
}
```

Harness chỉ accept artifact khi version supported, nonce đúng và address parse được; PID chỉ là
diagnostic/cross-check với child do harness sở hữu. Sau đó harness mới bounded-poll `/ready`. File
tồn tại không đồng nghĩa server ready.

Startup sequence bắt buộc:

```text
remove old bound-address artifact
→ generate startup nonce
→ spawn production binary
→ listener bind thành công
→ exclusive-create temporary sibling cùng directory
→ write + flush
→ atomic rename tới destination chưa tồn tại
→ optional fsync parent directory
→ server serve(listener)
→ harness validate artifact
→ bounded /ready polling
```

Destination artifact đã tồn tại trả `startup_handshake_artifact_exists` và startup fail; server
không overwrite. Temporary sibling cũng được tạo exclusive và không follow/claim file có sẵn. Nếu
write, flush hoặc rename thất bại sau bind, server đóng listener và fail startup, tuyệt đối không
serve. Server không xóa artifact khi shutdown; harness sở hữu stale-file cleanup trước mỗi spawn.

Controlled restart có authority cố định:

```text
SIGTERM → bounded wait process exit → assert exited → remove artifact
→ spawn production binary mới với nonce mới → wait bound address → probe /ready
```

Không exit trong deadline là qualification failure. Harness có thể force-kill chỉ để teardown
environment, nhưng không được báo controlled shutdown thành công.

Controlled-restart Mandatory Qualification V1 chỉ hỗ trợ Unix. Linux là CI authority; non-Unix
trả `UNAVAILABLE` với stable reason trước spawn, không dùng hard kill để giả lập SIGTERM.

### 30.2 Fresh create-only provisioning

Automated scenario provision là create-only và fail-closed. Conflict ở bất kỳ public resource
identity nào là failure; không reconcile, resume, claim hay adopt resource tồn tại. Harness dùng
fresh temporary SQLite cho Mandatory Qualification. Manual/shared-server mode vẫn chỉ soft-disable
cleanup, nhưng không biến create-only scenario thành workflow resume.

### 30.3 Deadline và retry policy

Global Mandatory Qualification deadline là hard upper bound và luôn thắng mọi stage deadline:

```text
effective_stage_deadline
    = min(stage_deadline, mandatory_global_remaining)
```

Defaults và validation:

```text
startup handshake overall  = 30 s, configurable 1..=300 s
readiness overall          = 30 s, configurable 1..=300 s
readiness poll interval    = 100 ms
single readiness probe     = min(1 s, readiness remaining, global remaining)
normal Admin request       = 10 s
controlled process exit    = shutdown.grace_ms + 5 s, maximum 65 s
force-kill reap            = 5 s
mandatory global           = 5 min, configurable 30..=1800 s
```

Nếu global deadline hết trong readiness, Provider Test, Voice turn hoặc controlled shutdown,
harness cancel stage hiện tại, teardown và fail `qualification_deadline_exceeded`. Child exit trước
handshake hoặc Ready làm stage fail ngay. Một logical mutation, Provider Test, MCP call hoặc Voice
turn có đúng một attempt; không retry conflict, busy, timeout hay transport failure. Chỉ readiness
GET được poll lặp, với mỗi probe có timeout riêng như trên.

### 30.4 Qualification result và report

Mandatory result là `PASS | FAIL | UNAVAILABLE`, tương ứng process exit code `0 | 1 | 2`.
`UNAVAILABLE` chỉ dùng khi prerequisite/platform không thể chạy gate, ví dụ
`unsupported_platform`; nó không downgrade assertion, runtime hay cleanup failure. Linux CI chỉ
chấp nhận `PASS`.

Report JSON là versioned, privacy-safe và create-new qua atomic write; không silently overwrite:

```json
{
  "schema_version": 1,
  "mandatory_result": "PASS",
  "server_build_profile": "qualification",
  "elapsed_ms": 12345,
  "stages": [
    {
      "name": "provider_runtime_verify",
      "status": "PASS",
      "code": "provider_runtime_verified",
      "elapsed_ms": 52,
      "request_id": null
    }
  ],
  "optional_evidence": [],
  "cleanup": {}
}
```

`code` là stable machine-readable identifier; `name` chỉ phục vụ human readability. Stage chỉ có
status/code/elapsed time và optional Admin request ID. Report không chứa prompt, transcript, tool
arguments/results, audio, token, `secret_ref`, startup nonce, temp path hay raw server response.
Optional Runtime Evidence dùng `PASS | FAIL | NOT_RUN` trong section riêng và không thay mandatory
result. `server_build_profile` là observation không nhạy cảm, không phải security authority. Stdout
chỉ in summary.

### 30.5 Teardown và artifact retention

Harness luôn chạy finally-style teardown để stop/reap mọi production child và mock. Token-bearing
config, environment snapshot và temporary secret material luôn bị xóa, kể cả khi main flow fail
hoặc `--keep-debug-artifacts-on-failure` được bật. Mặc định temp DB, handshake artifact,
intermediate state và debug artifacts đều bị xóa; chỉ versioned privacy-safe report được giữ.

Opt-in giữ debug artifacts không được override secret/token cleanup và chỉ giữ artifact đã qua
privacy allowlist. Report ghi category artifact đã giữ/xóa, không ghi absolute path.

Cleanup giữ riêng root cause:

```text
primary_failure = voice_turn_timeout
cleanup_failure = child_reap_failed
mandatory_result = FAIL
```

Cleanup failure không thay `primary_failure`. Nếu main stages đã pass nhưng teardown để lại child
process, secret material hoặc token-bearing file thì final result vẫn là `FAIL`. Lỗi xóa một
privacy-safe debug artifact được ghi riêng; CI có thể áp policy fail chặt hơn nhưng không được che
root cause ban đầu.

---

## 31. Không query SQLite trực tiếp

Reference client không được:

```text
open .db
run SQL
inspect table trực tiếp
```

Nó phải test public contracts qua:

```text
Admin API
Provider Diagnostic API
Voice WS
```

Direct DB test thuộc server-side test suite.

---

## 32. Logging và privacy

Không log:

```text
Admin token
secret value
Authorization header
MCP credential
raw audio body
provider secret
```

Có thể log:

```text
resource type
resource id
key
revision
HTTP status
server request_id
runtime_status
```

Nếu Admin API trả `X-Request-Id`, client nên capture để error report dễ trace.

---

## 33. Implementation phases

### RC-DB1 — Admin HTTP foundation

Thêm:

```text
AdminClient
AdminCredential
AdminError
response/error envelope
request-id capture
independent wire DTO boundary
```

Gate:

```text
GET/list/get hoạt động
401/403 map đúng
```

### RC-DB2 — Typed CRUD

Thêm typed APIs cho:

```text
Provider
Template
Agent
Device
MCP Server
```

Gate:

```text
create/get/list từng resource
```

### RC-DB3 — Bindings + revision

Thêm:

```text
Template → Provider
Agent → Template
Agent → MCP
Device → Agent
If-Match
409 stale revision
```

Gate:

```text
full resource graph tạo được
```

### RC-DB4 — Scenario provisioning

Thêm:

```text
ScenarioSpec
ScenarioState
scenario provision
```

Gate:

```text
fresh DB
→ full graph persisted
→ state file generated
→ RESTART_REQUIRED
```

### RC-DB5 — Runtime verify + Provider diagnostics

Dependency bắt buộc:

```text
Provider Descriptor/Test API public đã được implement
qualification-providers feature và descriptors đã được implement
```

Không tạo diagnostic seam riêng trong Reference Integration Client.

Thêm:

```text
scenario verify
runtime status
ASR test
LLM test
TTS test
```

Gate:

```text
after restart
→ providers loaded
→ runtime_matches_desired=true
→ deterministic public Provider Test API pass
→ không cần external credential/model
```

Optional evidence chạy real ASR/LLM/TTS provider bằng cùng public API và report `PASS | FAIL | NOT_RUN` riêng; nó không thay đổi gate RC-DB5.

### RC-DB6 — Voice + External MCP E2E

Thêm:

```text
mock Streamable HTTP MCP
fresh discovery verification
Voice smoke
IntegrationHarness process lifecycle
```

Gate:

```text
Device
→ Agent
→ Template
→ Provider runtime
→ MCP discovery
→ Voice turn
→ TTS complete
```

---

## 34. Definition of Done

Mandatory Qualification:

```text
[ ] AdminClient typed
[ ] Wire DTO độc lập với server implementation types
[ ] Admin Bearer lấy từ env
[ ] Không dùng raw JSON cho core CRUD
[ ] Revision/If-Match được hỗ trợ
[ ] Stale revision → 409
[ ] ScenarioSpec declarative
[ ] ScenarioState persist qua restart
[ ] ScenarioState dùng key/device_id + latest revision làm authority
[ ] Numeric DB ID chỉ là optional diagnostic metadata
[ ] Raw TOML SHA-256 được kiểm tra trước side effect ở verify/voice-smoke/disable
[ ] Endpoint cũ không thuộc ScenarioState authority
[ ] Full Provider/Template/Agent/MCP/Device graph provision được
[ ] Unknown Device → 403 trước provisioning
[ ] Provision report RESTART_REQUIRED
[ ] Verify kiểm tra runtime sau restart
[ ] Deterministic ASR/LLM/TTS Provider Test API pass
[ ] Mandatory gate không cần external credential/model
[ ] Default build không chứa qualification adapters; qualification build chứa đủ VAD/ASR/LLM/TTS
[ ] Qualification adapters đi qua DB load plan/RuntimeCatalog, không inject ProviderSet/AppState
[ ] Voice smoke tái sử dụng WS client hiện tại
[ ] Mock External MCP hỗ trợ initialize/tools/list/tools/call
[ ] MCP fresh discovery được chứng minh
[ ] Mock MCP first-256 ordered event log không overflow
[ ] Device MCP và External MCP tách namespace/flow
[ ] Không log token/secret
[ ] Không query SQLite trực tiếp
[ ] Không hot-load Provider trong test flow
[ ] Automated mode dùng temp SQLite
[ ] IntegrationHarness là Mandatory Qualification authority trong CI
[ ] AdminClient và ScenarioRunner không sở hữu process lifecycle
[ ] Production binary publish exclusive nonce-bound startup artifact trước serve
[ ] Controlled SIGTERM restart pass trên Linux CI; non-Unix trả UNAVAILABLE trước spawn
[ ] Global qualification deadline giới hạn mọi stage và readiness probe
[ ] Mutation/diagnostic/Voice operation không retry
[ ] Versioned create-new report dùng PASS/FAIL/UNAVAILABLE và exit 0/1/2
[ ] Finally-style teardown giữ primary và cleanup failure riêng
[ ] Secret/token cleanup không thể bị debug-retention flag override
[ ] Shared-server cleanup chỉ soft-disable
```

Optional Runtime Evidence:

```text
[ ] Real ASR model: PASS | FAIL | NOT_RUN
[ ] Real LLM remote API: PASS | FAIL | NOT_RUN
[ ] Real TTS model/provider: PASS | FAIL | NOT_RUN
[ ] Real External MCP: PASS | FAIL | NOT_RUN
[ ] Hardware/HIL: PASS | FAIL | NOT_RUN
```

Các mục optional không phải điều kiện để Mandatory Qualification pass.

---

## 35. Full reference flow

Mandatory Qualification:

```text
Fresh SQLite
    ↓
Server boot + migrations
    ↓
unknown Device rejected
    ↓
Rust client provision
    ├── Providers
    ├── Template
    ├── Template→Provider bindings
    ├── Agent
    ├── Agent→Template
    ├── MCP
    ├── Agent→MCP
    └── Device→Agent
    ↓
RESTART_REQUIRED
    ↓
Server restart
    ↓
RuntimeCatalog rebuilt
    ↓
Rust client verify
    ├── runtime_status
    ├── runtime_matches_desired
    ├── deterministic ASR Provider Test API
    ├── deterministic LLM Provider Test API
    └── deterministic TTS Provider Test API
    ↓
Mock External MCP
    ↓
Rust Voice smoke
    ↓
DB admission
    ↓
Agent
    ↓
Default Template
    ↓
Provider runtime
    ↓
External MCP initialize/tools/list
    ↓
LLM/TTS
    ↓
TTS complete
    ↓
PASS
```

Optional Runtime Evidence chạy sau hoặc độc lập với mandatory flow:

```text
real providers / remote services / hardware
    ↓
PASS | FAIL | NOT_RUN
    ↓
không thay đổi mandatory qualification result
```

---

## 36. Kết luận

Rust reference client sau khi mở rộng phải kiểm thử được cả:

```text
Control Plane
    Admin API
    SQLite desired state
    bindings
    optimistic concurrency

Data Plane
    Voice admission
    Agent/Template resolution
    Provider runtime
    External MCP discovery
    TTS lifecycle
```

Invariant quan trọng nhất:

```text
Provisioning ≠ Runtime activation
```

Và qualification invariant:

```text
Mandatory deterministic qualification ≠ Optional real-environment evidence
```

Do đó flow reference chính thức là:

```text
provision
→ restart
→ verify
→ voice-smoke
```
