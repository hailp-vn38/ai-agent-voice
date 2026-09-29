# Hướng dẫn tích hợp SQLite cho Agent / Template / Provider / MCP / Device / History

> Baseline: `hailp-vn38/ai-agent-voice`, branch `dev-test`, kiểm tra ngày 2026-09-28.
>
> Mục tiêu: thêm SQLite vào server Rust mà không đưa database vào audio hot path; hỗ trợ Agent ↔ Template N:N, switch Template trong cùng WebSocket session, Provider theo Template, MCP Streamable HTTP theo Agent, Device → Agent, và history text theo `session_id`.

---

## 1. Mục tiêu kiến trúc

Database chỉ lưu **persistent control-plane state**:

- Agent.
- Template Agent.
- quan hệ Agent ↔ Template.
- Provider instance và cấu hình provider.
- binding Template → VAD/ASR/LLM/TTS.
- MCP Streamable HTTP server và binding Agent → MCP.
- Device và binding Device → Agent.
- text history của hội thoại, group bằng `session_id` của WebSocket connection.
- bounded metadata audit của authenticated admin mutation.

Database **không** lưu realtime state:

- PCM/audio buffer.
- VAD state.
- ASR stream state.
- TTS packet/queue.
- pending MCP request.
- current generation gate.
- active turn permit.
- live WebSocket state.

Các state trên vẫn thuộc `SessionActor`, worker runtime và WebSocket writer.

### 1.1. Model domain

```text
Agent
  = identity của assistant

TemplateAgent
  = runtime behavior/profile
  = prompt + language + provider bindings

Provider
  = concrete provider instance
  = adapter + typed/config JSON

Device
  = physical/logical client
  = bind tới một Agent

MCP Server
  = external Streamable HTTP MCP configuration
  = bind tới Agent

HistoryMessage
  = final user/assistant text
  = group theo WebSocket session_id
```

### 1.2. ERD mức cao

```text
agents
   │
   ├──────── N:N ──────── agent_templates
   │                         │
   │                         │ 1:N
   │                         ▼
   │              template_provider_bindings
   │                         │
   │                         ▼
   │                     providers
   │
   ├──────── N:N ──────── mcp_servers
   │
   └──────── 1:N ──────── devices
                              │
                              │ 1:N
                              ▼
                       history_messages
                              │
                              └── GROUP BY session_id
```

---

# 2. Quyết định bắt buộc trước implementation

## D1 — SQLite + SQLx

Dùng:

```text
SQLite
SQLx 0.8
Tokio
Axum
```

Không thêm PostgreSQL/Redis cho phase này.

SQLite chạy WAL mode và database nằm local trên homelab server.

## D2 — `config.toml` vẫn giữ server defaults

`config.toml` tiếp tục chứa:

- deployment/runtime configuration;
- worker limits;
- server defaults;
- `[provider_defaults]`;
- provider instances mặc định cần server bootstrap;
- secrets qua env/config hiện có nếu cần.

SQLite bổ sung runtime/business configuration.

Rule resolve:

```text
Device
  → Agent
     → default Template nếu Agent có Template
        → provider bindings của Template

Agent không có bất kỳ Template assignment nào
  → fallback toàn bộ EffectiveAgentConfig + provider_defaults của server
```

Không fallback từng provider riêng lẻ bên trong một Template.

Template đã được activate phải là một profile hợp lệ và đầy đủ:

```text
prompt
language
VAD
ASR
LLM
TTS
```

Nếu thiếu một binding provider bắt buộc thì Template không được activate.

## D3 — Active Template là session state, không phải Agent state

Không thêm:

```text
agents.active_template_id
```

vì hai device hoặc hai WebSocket connection có thể dùng cùng Agent nhưng active Template khác nhau.

Active Template nằm trong `SessionActor`:

```rust
SessionTemplateState {
    active_template_id: Option<i64>,
    pending_template_id: Option<i64>,
    revision: u64,
}
```

## D4 — Template switch chỉ có hiệu lực từ turn kế tiếp

Không đổi prompt/provider giữa một LLM operation/tool continuation.

Flow chuẩn:

```text
Turn N / Template A
  User
  LLM
  ToolCall switch_template(B)
  ToolResult success
  LLM continuation vẫn dùng Template A
  TTS A
  Writer TurnClosed(Normal)

          ↓ commit switch

Turn N+1 / Template B
```

Điều này giữ LLM Base Snapshot và provider selection ổn định trong toàn bộ turn.

## D5 — Persistent Transcript là opt-in và chỉ lưu text cuối cùng

Mặc định persistence transcript tắt: `database.history.enabled = false`. Khi tắt,
runtime không được enqueue hoặc write vào `history_messages` theo bất kỳ đường nào.
Schema/migration tồn tại không tự bật feature này.

Không persist:

- system prompt;
- tool call arguments;
- tool results;
- ASR partial;
- LLM delta;
- audio;
- TTS chunks.

Persist:

```text
role=user      + final user text
role=assistant + delivered final assistant text
```

Assistant chỉ persist sau `WriterEvent::TurnClosed { outcome: Normal }`.

Khi bật, transcript chỉ được đọc qua admin API đã xác thực. Retention dùng
`retention_days`; cleanup chạy ngoài realtime path. Không log nội dung transcript,
prompt hoặc secret trong tracing. ADR-0048 supersede riêng phần cấm persistent
transcript của ADR-0024; các nguyên tắc privacy khác của ADR-0024 vẫn giữ nguyên.

## D6 — Không có bảng `device_sessions`

Mỗi WebSocket connection sử dụng `session_id` hiện có của server.

`history_messages` chứa trực tiếp:

```text
session_id
device_id
agent_id
template_id
```

Các message cùng phiên có cùng `session_id`.

## D7 — Schema không tự đổi Voice semantics

Database availability, migration và schema không được âm thầm đổi Voice semantics.
Mỗi behavior mới chỉ active ở phase tương ứng và khi config enable explicit. DB-1 và
DB-2 không đổi WebSocket admission, provider resolution, Agent, Template, MCP hay
history runtime.

## D8 — Effective Session Profile immutable đến disconnect

Admission resolve đúng một `EffectiveSessionProfile`; `SessionActor` chỉ giữ snapshot
này, không giữ DB row hoặc live repository reference. Admin mutation chỉ tác động
Voice Session mới; muốn revoke session đang chạy phải là feature command/event riêng,
không là side effect của DB mutation.

Snapshot gồm `TemplateSwitchCatalog` của các candidate enabled đã validate; tool switch
chỉ lookup catalog này, không query SQLite.

Agent không có assignment dùng ServerDefaultProfile với `TemplateSwitchCatalog {}`;
không advertise `server.switch_template`, thay vì expose capability rồi fail runtime.

---

# 3. Dependencies

Thêm vào workspace `Cargo.toml`:

```toml
[workspace.dependencies]
sqlx = { version = "0.8", default-features = false, features = [
    "runtime-tokio",
    "sqlite",
    "migrate",
    "macros",
    "uuid",
    "json"
] }
```

Thêm vào:

```text
crates/voice-agent-server/Cargo.toml
```

```toml
[dependencies]
sqlx.workspace = true
```

Không dùng synchronous `rusqlite` trong async request/session path.

---

# 4. Database configuration

Thêm vào `AppConfig`:

```rust
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatabaseConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,

    #[serde(default = "default_database_url")]
    pub url: String,

    #[serde(default = "default_database_max_connections")]
    pub max_connections: u32,

    #[serde(default = "default_database_busy_timeout_ms")]
    pub busy_timeout_ms: u64,

    #[serde(default = "default_true")]
    pub migrate_on_start: bool,

    #[serde(default)]
    pub devices: DatabaseDevicesConfig,

    #[serde(default)]
    pub history: DatabaseHistoryConfig,

    #[serde(default)]
    pub audit: DatabaseAuditConfig,

}
```

```rust
pub struct ShutdownConfig {
    pub grace_ms: u64,
}
```

`AppConfig`:

```rust
pub struct AppConfig {
    // ...
    #[serde(default)]
    pub database: DatabaseConfig,

    #[serde(default)]
    pub api: ApiConfig,

    #[serde(default)]
    pub shutdown: ShutdownConfig,
}
```

Config mẫu:

```toml
[database]
enabled = true
url = "sqlite://data/voice-agent.db"
max_connections = 5
busy_timeout_ms = 5000
migrate_on_start = true

[shutdown]
grace_ms = 15000

[database.devices]
# Chỉ được áp dụng từ DB-4 khi database-backed admission đã được enable.
admission_enabled = false
auto_register = false
auto_register_agent_key = ""

[database.history]
# Chỉ được áp dụng từ DB-8.
enabled = false
retention_days = 30

[database.audit]
retention_days = 90

[mcp.external]
per_server_resolution_timeout_ms = 3000
overall_resolution_budget_ms = 5000
max_concurrent_calls_per_server = 16

[mcp.external.limits]
max_tools_per_server = 128
max_tools_per_session = 512
max_tool_schema_bytes = 16384
max_tool_description_bytes = 4096
max_external_tool_result_bytes = 16384
max_pages_per_server = 32

[mcp.external.network]
allow_http_lan = false
allowed_hosts = []
allowed_cidrs = []

[llm.tools]
max_calls_per_round = 8
max_rounds_per_turn = 4
execution_budget_ms = 30000

[api]
enabled = false
admin_token = ""
```

Với homelab không cần pool lớn. Bắt đầu bằng `5` là đủ.

Validation `ApiConfig`: khi `api.enabled = true`, `admin_token.trim()` bắt buộc
non-empty, nếu không startup fail-fast. Khi disabled, token có thể rỗng và admin
router không được mount (client nhận `404`).

Validation `DatabaseHistoryConfig`: `retention_days` bắt buộc trong `1..=365`;
không hỗ trợ `0`, giá trị âm, `null`, hay unlimited.

Validation `DatabaseAuditConfig`: `retention_days` bắt buộc trong `30..=3650`, độc lập
với history retention.

Validation `DatabaseConfig.busy_timeout_ms`: `1..=30_000`, default `5_000`; `0` không được
phép vì biến lock contention ngắn thành failure tức thời.

Validation `ShutdownConfig.grace_ms`: `1_000..=60_000`, default `15_000` ms.

Validation External MCP: timeout/budget/limit phải dương; overall budget không nhỏ hơn
timeout một server. `max_concurrent_calls_per_server` là `1..=64`, default `16`, và
giới hạn semaphore global theo MCP server cho mọi session trong process. Operator chỉ cấu
hình trong hard ceiling: tools/server `<=512`, tools/session `<=2048`, schema/result
`<=65536` bytes, description `<=16384` bytes.

Validation `mcp.external.network`: destination bắt buộc match `allowed_hosts` hoặc
`allowed_cidrs`; mặc định chỉ HTTPS. HTTP chỉ được phép khi `allow_http_lan=true` và
vẫn match allowlist. URL không có userinfo, fragment hoặc query string; redirects tắt
trong V1. Validate hostname allowlist rồi resolve DNS ngay trước connect và mọi resolved
IP cũng phải pass policy để chống DNS rebinding.

Validation `llm.tools` tập trung ở config layer, trước bind listener:
`max_calls_per_round ∈ 1..=32`, `max_rounds_per_turn ∈ 1..=8` và
`execution_budget_ms ∈ 1..=120_000`. Default lần lượt là `8`, `4` và `30_000` ms;
`0`, số âm nếu schema/parser biểu diễn được, hoặc giá trị vượt hard ceiling đều là
configuration error fail startup. `max_calls_per_round` và `max_rounds_per_turn` là
policy cap, không queue capacity; `SessionActor` không lặp lại validation config này.

`database.devices.admission_enabled = true` yêu cầu `database.enabled = true`; khi
database disabled, admin routes, history persistence và database-backed admission đều
không active.

## 4.1. SQLite deployment và fresh bootstrap

V1 chỉ hỗ trợ đúng một Voice Agent process owner cho mỗi SQLite database path, trên local
filesystem. NFS/SMB/shared volume, active-active hoặc multi-process writer không supported;
không thêm migration leader election hay cross-process write coordination trong V1.

Migration chỉ tạo schema/index, không seed ngầm Agent, Template hoặc Device. Dev seed phải là
thao tác explicit. Khi database-backed admission bật trên DB mới chưa provision Device, unknown
Device trả `403` theo admission contract.

---

# 5. Database module layout

Tạo module riêng, không đặt SQL trực tiếp trong handler Axum hoặc `SessionActor`.

```text
crates/voice-agent-server/src/database/
├── mod.rs
├── connection.rs
├── error.rs
├── models/
│   ├── mod.rs
│   ├── agent.rs
│   ├── template.rs
│   ├── provider.rs
│   ├── mcp.rs
│   ├── device.rs
│   ├── history.rs
│   └── audit.rs
├── repositories/
│   ├── mod.rs
│   ├── agent.rs
│   ├── template.rs
│   ├── provider.rs
│   ├── mcp.rs
│   ├── device.rs
│   ├── history.rs
│   └── audit.rs
├── query/
│   ├── mod.rs
│   ├── pagination.rs
│   ├── filter.rs
│   └── sort.rs
└── service/
    ├── mod.rs
    ├── agent_config.rs
    ├── template_resolver.rs
    ├── history.rs
    └── audit.rs
```

Migrations:

```text
crates/voice-agent-server/migrations/
├── 0001_initial_database.sql
├── 0002_indexes.sql
└── ...
```

Migration history do `sqlx::migrate!()` quản lý là authoritative; không thêm version counter
song song. Schema là monotonic forward-only: binary phải kiểm tra migration version đã apply
không lớn hơn migration embedded mới nhất của chính binary **trước** apply pending migration.
DB mới hơn binary là `database_schema_incompatible` và startup fail trước listener. Không
ignore schema lạ, reverse migration, drop column/table hoặc best-effort boot.

Expose:

```rust
pub mod database;
```

Secret resolution không thuộc database module. Application bootstrap sở hữu seam riêng:

```rust
pub trait SecretResolver: Send + Sync {
    fn resolve(&self, secret_ref: &SecretRef) -> Result<SecretValue, SecretResolveError>;
}

pub struct SecretRef(String);

impl SecretRef {
    pub fn parse(value: String) -> Result<Self, SecretRefError> {
        let bytes = value.as_bytes();
        if bytes.is_empty() || bytes.len() > 256 {
            return Err(SecretRefError::InvalidLength);
        }
        if bytes.iter().any(|b| !(0x20..=0x7e).contains(b)) {
            return Err(SecretRefError::InvalidCharacter);
        }
        if bytes.iter().all(|b| *b == b' ') {
            return Err(SecretRefError::Blank);
        }
        if bytes.first() == Some(&b' ') || bytes.last() == Some(&b' ') {
            return Err(SecretRefError::SurroundingWhitespace);
        }
        Ok(Self(value))
    }

    pub(crate) fn as_str(&self) -> &str { &self.0 }
}

impl std::fmt::Debug for SecretRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretRef([REDACTED])")
    }
}

pub struct SecretValue(zeroize::Zeroizing<String>);

impl SecretValue {
    pub fn expose(&self) -> &str { &self.0 }
}

impl std::fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretValue([REDACTED])")
    }
}
```

`SecretRef` là opaque printable ASCII `1..=256` bytes (`0x20..=0x7E`), reject empty,
whitespace-only, leading/trailing space và mọi control/non-ASCII byte; parser không trim hay
normalize. `EnvSecretResolver` là implementation V1: chỉ backend này đọc `std::env` và áp
env-name syntax, trả `SecretResolveError::Invalid`/`secret_invalid` cho ref domain-valid nhưng
không phải env name. Trait cho phép thay Vault, Docker/File secret, cloud resolver hoặc
composite resolver mà không đổi schema. `SecretRef` và `SecretValue` không implement `Display`
hoặc `Clone`; SecretValue storage zeroize khi drop và value/reference không đi vào audit,
metrics, logs, history hay Session Profile Revision.

---

# 6. Connection layer

`database/connection.rs` chịu trách nhiệm duy nhất về:

- open pool;
- SQLite options;
- WAL;
- foreign keys;
- busy timeout;
- migrations.

Ví dụ contract:

```rust
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};
use std::{str::FromStr, time::Duration};

#[derive(Clone)]
pub struct Database {
    pool: SqlitePool,
}

impl Database {
    pub async fn connect(config: &DatabaseConfig) -> Result<Self, DatabaseError> {
        let options = SqliteConnectOptions::from_str(&config.url)?
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(Duration::from_millis(config.busy_timeout_ms));

        let pool = SqlitePoolOptions::new()
            .max_connections(config.max_connections)
            .connect_with(options)
            .await?;

        let migrator = sqlx::migrate!("./migrations");
        ensure_schema_not_newer_than_binary(&pool, &migrator).await?;
        if config.migrate_on_start {
            migrator.run(&pool).await?;
        } else {
            ensure_schema_is_current(&pool, &migrator).await?;
        }

        Ok(Self { pool })
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}
```

Không gọi PRAGMA rải rác trong repositories.

## 6.1. SQLite contention và pool exhaustion

`PRAGMA busy_timeout` chỉ là wait mechanism cho SQLite lock (`SQLITE_BUSY`/`SQLITE_LOCKED`),
không áp dụng cho `SqlitePool::acquire()` exhaustion. V1 không có application retry loop,
exponential backoff, retry transaction hoặc retry admission query. Internal taxonomy phải tách:

```text
database_busy         = SQLITE_BUSY / SQLITE_LOCKED sau busy_timeout
database_pool_timeout = không acquire được SQLx pool connection
database_unavailable  = I/O, connection hoặc storage failure khác
```

Admin GET/PATCH/POST gặp `database_busy` trả `503` envelope code `database_busy`; pool timeout
và unavailable trả `503 database_unavailable` (telemetry vẫn giữ taxonomy gốc), không raw SQLite
message. DB-backed WS admission gặp một trong các lỗi này trả `503` trước upgrade, không fallback
hay retry handler. Session đã admit tiếp tục snapshot immutable.

HistoryWriter gặp busy/unavailable/pool timeout drop record theo existing `database_error`
metric reason, không requeue/backlog retry. Retention/audit cleanup gặp contention abort current
iteration, emit metric/coarse log, và chỉ thử lại ở scheduled 24-hour run sau. Không `while busy`
sleep/retry.

`SessionActor` không acquire/retry SQLite: admission chạy trước actor, history chỉ `try_send` vào
bounded writer, còn Admin API ở ngoài realtime actor. WAL cải thiện reader/writer concurrency
nhưng không loại write serialization. Transaction chỉ chứa DB work atomic (revision check,
mutation, revision increment, success audit insert); không giữ transaction qua HTTP/MCP/provider/
filesystem hay await external service.

---

# 7. Schema SQL

## 7.1. `agents`

```sql
CREATE TABLE agents (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    key             TEXT NOT NULL UNIQUE,
    name            TEXT NOT NULL,
    description     TEXT,
    enabled         INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    revision        INTEGER NOT NULL DEFAULT 1 CHECK (revision > 0),
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);
```

Agent chỉ chứa thông tin cơ bản.

Không đặt prompt/language/provider trong bảng này.

---

## 7.2. `agent_templates`

```sql
CREATE TABLE agent_templates (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    key             TEXT NOT NULL UNIQUE,
    name            TEXT NOT NULL,
    description     TEXT,
    language        TEXT NOT NULL,
    prompt          TEXT NOT NULL,
    enabled         INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    revision        INTEGER NOT NULL DEFAULT 1 CHECK (revision > 0),
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);
```

Validation application-side:

- `key` không rỗng;
- `name` không rỗng;
- `language` không rỗng;
- `prompt` không rỗng;
- prompt size phải có hard bound tương thích prompt limits hiện tại.

---

## 7.3. `agent_template_assignments`

Quan hệ N:N:

```sql
CREATE TABLE agent_template_assignments (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id        INTEGER NOT NULL,
    template_id     INTEGER NOT NULL,
    is_default      INTEGER NOT NULL DEFAULT 0 CHECK (is_default IN (0, 1)),
    enabled         INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    created_at      INTEGER NOT NULL,

    FOREIGN KEY (agent_id) REFERENCES agents(id) ON DELETE CASCADE,
    FOREIGN KEY (template_id) REFERENCES agent_templates(id) ON DELETE CASCADE,

    UNIQUE(agent_id, template_id)
);

CREATE UNIQUE INDEX idx_agent_one_default_template
ON agent_template_assignments(agent_id)
WHERE is_default = 1 AND enabled = 1;
```

Rule:

- mỗi Agent tối đa một default Template enabled;
- Template có thể assign cho nhiều Agent;
- Agent có thể có nhiều Template.

---

## 7.4. `providers`

Provider row là **provider instance**, không chỉ là adapter type.

```sql
CREATE TABLE providers (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    key             TEXT NOT NULL UNIQUE,
    name            TEXT NOT NULL,
    type            TEXT NOT NULL CHECK (type IN ('vad', 'asr', 'llm', 'tts')),
    adapter         TEXT NOT NULL,
    config_json     TEXT NOT NULL CHECK (json_valid(config_json)),
    secret_ref      TEXT,
    enabled         INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    revision        INTEGER NOT NULL DEFAULT 1 CHECK (revision > 0),
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL
);
```

`config_json` là canonical serialization của typed **non-secret** adapter config, không phải
bag JSON tùy ý từ client. Credential Provider chỉ đi qua `secret_ref → SecretResolver →
SecretValue`; không adapter nào được đọc plaintext credential từ `config_json`.

Ví dụ:

```text
key     = gipformer_vi
type    = asr
adapter = gipformer_sherpa_offline
```

```json
{
  "model": "gipformer1.5-68M-rnnt",
  "num_threads": 2,
  "provider": "cpu"
}
```

Không lưu plaintext secret trong `config_json`.

```text
secret_ref = OPENAI_PRIMARY_API_KEY
```

`secret_ref` opaque ở domain/API layer. Bootstrap inject `Arc<dyn SecretResolver>` vào
`AppState`; Provider loader và External MCP client chỉ gọi trait này, không đọc environment
trực tiếp. V1 dùng `EnvSecretResolver`, nơi resolver backend mới diễn giải reference là
tên environment variable. SQLite không biết backend hay secret value.

---

## 7.5. `template_provider_bindings`

```sql
CREATE TABLE template_provider_bindings (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    template_id     INTEGER NOT NULL,
    provider_type   TEXT NOT NULL CHECK (provider_type IN ('vad', 'asr', 'llm', 'tts')),
    provider_id     INTEGER NOT NULL,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,

    FOREIGN KEY (template_id) REFERENCES agent_templates(id) ON DELETE CASCADE,
    FOREIGN KEY (provider_id) REFERENCES providers(id) ON DELETE RESTRICT,

    UNIQUE(template_id, provider_type)
);
```

Application validation bắt buộc kiểm tra:

```text
binding.provider_type == providers.type
```

Không cho bind provider `tts` vào slot `asr`.

Template được activate chỉ khi có đủ:

```text
vad
asr
llm
tts
```

---

## 7.6. `mcp_servers`

MCP ở đây là **server-side external MCP Streamable HTTP**, tách khỏi Device MCP hiện có qua WebSocket.

```sql
CREATE TABLE mcp_servers (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    key                 TEXT NOT NULL UNIQUE,
    name                TEXT NOT NULL,
    transport           TEXT NOT NULL DEFAULT 'streamable_http'
                        CHECK (transport = 'streamable_http'),
    url                 TEXT NOT NULL,
    headers_json        TEXT NOT NULL DEFAULT '{}'
                        CHECK (json_valid(headers_json)),
    auth_type           TEXT NOT NULL DEFAULT 'none'
                        CHECK (auth_type IN ('none', 'bearer', 'header')),
    auth_header_name    TEXT,
    secret_ref          TEXT,
    connect_timeout_ms  INTEGER NOT NULL DEFAULT 5000 CHECK (connect_timeout_ms > 0),
    request_timeout_ms  INTEGER NOT NULL DEFAULT 30000 CHECK (request_timeout_ms > 0),
    enabled             INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    revision            INTEGER NOT NULL DEFAULT 1 CHECK (revision > 0),
    created_at          INTEGER NOT NULL,
    updated_at          INTEGER NOT NULL
);
```

`headers_json` chỉ chứa non-secret headers/config. Auth là typed: `none` không có
secret; `bearer` inject `Authorization: Bearer <resolved secret>`; `header` inject
`<auth_header_name>: <resolved secret>`. `header` không hỗ trợ prefix, suffix, template
value, multiple auth header hoặc query-string auth.

---

## 7.7. `agent_mcp_bindings`

```sql
CREATE TABLE agent_mcp_bindings (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id        INTEGER NOT NULL,
    mcp_server_id   INTEGER NOT NULL,
    enabled         INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    required        INTEGER NOT NULL DEFAULT 0 CHECK (required IN (0, 1)),
    created_at      INTEGER NOT NULL,

    FOREIGN KEY (agent_id) REFERENCES agents(id) ON DELETE CASCADE,
    FOREIGN KEY (mcp_server_id) REFERENCES mcp_servers(id) ON DELETE CASCADE,

    UNIQUE(agent_id, mcp_server_id)
);
```

Trong V1, một binding enabled publish toàn bộ tool mà server đó discover, validate và
đưa vào Session Tool Catalog thành công; không có allowlist/denylist từng tool trên
binding. Tool-level authorization là phase riêng sau này, không được suy diễn từ thứ tự
bind hoặc tên tool.

V1 chỉ hỗ trợ `required = false`; field này giữ đường mở rộng để một phase sau có thể
reject `503` khi required MCP unavailable mà không đổi schema.

MCP thuộc Agent, không thuộc active Template.

Switch Template không tự disconnect/reconnect MCP Agent bindings.

---

## 7.8. `devices`

```sql
CREATE TABLE devices (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    device_id       TEXT NOT NULL UNIQUE,
    agent_id        INTEGER NOT NULL,
    name            TEXT,
    description     TEXT,
    enabled         INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    metadata_json   TEXT CHECK (metadata_json IS NULL OR json_valid(metadata_json)),
    revision        INTEGER NOT NULL DEFAULT 1 CHECK (revision > 0),
    last_seen_at    INTEGER,
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,

    FOREIGN KEY (agent_id) REFERENCES agents(id) ON DELETE RESTRICT
);
```

`device_id` là protocol device identity.

Không dùng `client_id` làm persistent Device PK.

---

## 7.9. `history_messages`

Không tạo `device_sessions`.

```sql
CREATE TABLE history_messages (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id      TEXT NOT NULL,
    device_id       INTEGER NOT NULL,
    agent_id        INTEGER NOT NULL,
    template_id     INTEGER,
    sequence        INTEGER NOT NULL CHECK (sequence > 0),
    turn_id         TEXT,
    role            TEXT NOT NULL CHECK (role IN ('user', 'assistant')),
    text            TEXT NOT NULL,
    created_at      INTEGER NOT NULL,

    FOREIGN KEY (device_id) REFERENCES devices(id) ON DELETE CASCADE,
    FOREIGN KEY (agent_id) REFERENCES agents(id) ON DELETE RESTRICT,
    FOREIGN KEY (template_id) REFERENCES agent_templates(id) ON DELETE SET NULL,

    UNIQUE(session_id, sequence)
);
```

`template_id = NULL` có nghĩa session đang dùng server default vì Agent không có Template.

Không derive historical Agent từ `devices.agent_id`: `agent_id` phải được snapshot vào message.

---

## 7.10. `admin_audit_events`

```sql
CREATE TABLE admin_audit_events (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    created_at      INTEGER NOT NULL,
    request_id      TEXT NOT NULL,
    resource_type   TEXT NOT NULL,
    resource_id     INTEGER,
    action          TEXT NOT NULL,
    prior_revision  INTEGER,
    new_revision    INTEGER,
    outcome         TEXT NOT NULL,
    error_kind      TEXT,
    affected_rows   INTEGER
);
```

Audit success mutation (`agent.update`, `template.disable`, binding mutation,
`device.rebind_agent`, `mcp_server.update`, `history.purge`) và authenticated revision
conflict. Failed authentication chỉ telemetry, không DB audit. Không lưu prompt,
history text, config_json, secret_ref, resolved secret, HTTP Authorization, request
body, field diff hay arbitrary error string. Audit retention theo
`database.audit.retention_days`, độc lập transcript retention.

Audit maintenance chạy initial rồi mỗi 24 giờ, độc lập `history.enabled`, history
retention, last access và resource revision. Xóa batch/yield để tránh SQLite writer
lock dài, ví dụ mỗi batch 1.000 row cũ hơn `now_utc - retention_days`. Failure chỉ
metric + coarse structured log, không block Voice/admin mutation và không tạo audit
event cho chính audit cleanup.

---

# 8. Indexes

Migration index:

```sql
CREATE INDEX idx_devices_agent
ON devices(agent_id);

CREATE INDEX idx_agent_templates_agent
ON agent_template_assignments(agent_id, enabled, is_default);

CREATE INDEX idx_template_provider_template
ON template_provider_bindings(template_id, provider_type);

CREATE INDEX idx_provider_type_enabled
ON providers(type, enabled);

CREATE INDEX idx_agent_mcp_agent
ON agent_mcp_bindings(agent_id, enabled);

CREATE INDEX idx_history_device_time
ON history_messages(device_id, created_at DESC);

CREATE INDEX idx_history_agent_time
ON history_messages(agent_id, created_at DESC);

CREATE INDEX idx_history_template_time
ON history_messages(template_id, created_at DESC);

CREATE INDEX idx_history_session_turn
ON history_messages(session_id, turn_id);

CREATE INDEX idx_admin_audit_created_at
ON admin_audit_events(created_at DESC);
```

`UNIQUE(session_id, sequence)` đã tạo index cho ordering key; không cần duplicate index `(session_id, sequence)`.

---

# 9. Typed database models

DB row và API/domain object không nên là một type duy nhất.

Ví dụ:

```rust
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AgentRow {
    pub id: i64,
    pub key: String,
    pub name: String,
    pub description: Option<String>,
    pub enabled: bool,
    pub revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
}
```

Command object:

```rust
pub struct CreateAgent {
    pub key: String,
    pub name: String,
    pub description: Option<String>,
}
```

Không truyền `serde_json::Value` xuyên toàn bộ application layer; `provider.config_json` chỉ
được dùng transient ở `ProviderConfigValidator` trước khi chuyển thành typed adapter config.

---

# 10. Repository layer

## 10.1. Mục tiêu

Axum handler không viết SQL.

`SessionActor` không viết SQL.

Service layer không quản lý connection pool trực tiếp.

Flow:

```text
HTTP / WebSocket bootstrap
        ↓
Service
        ↓
Repository
        ↓
SQLx / SQLite
```

## 10.2. Repository contracts

Không cần trait cho mọi implementation nếu project chỉ dùng SQLite. Có thể dùng concrete repository wrapper để giảm boilerplate:

```rust
#[derive(Clone)]
pub struct AgentRepository {
    db: Database,
}

impl AgentRepository {
    pub async fn get_by_id(&self, id: i64) -> Result<Option<AgentRow>, DatabaseError>;
    pub async fn get_by_key(&self, key: &str) -> Result<Option<AgentRow>, DatabaseError>;
    pub async fn create(&self, input: CreateAgent) -> Result<AgentRow, DatabaseError>;
    pub async fn update(&self, id: i64, input: UpdateAgent) -> Result<AgentRow, DatabaseError>;
    pub async fn list(&self, query: AgentListQuery) -> Result<Page<AgentRow>, DatabaseError>;
}
```

Tương tự:

```text
TemplateRepository
ProviderRepository
McpRepository
DeviceRepository
HistoryRepository
```

---

# 11. Yêu cầu quan trọng: Query/API infrastructure dùng lại

Đây là phần phải xây từ đầu để sau này thêm API query database **không phải viết lại connection, pagination, sorting, error mapping và response envelope**.

Không tạo một endpoint kiểu:

```text
POST /api/sql
{ "query": "SELECT ..." }
```

Không expose raw SQL hoặc arbitrary column/filter từ client.

Thay vào đó tạo **typed reusable query layer**.

## 11.1. Common pagination

`database/query/pagination.rs`:

```rust
#[derive(Debug, Clone, serde::Deserialize)]
pub struct PageQuery {
    #[serde(default = "default_page")]
    pub page: u32,

    #[serde(default = "default_page_size")]
    pub page_size: u32,
}

impl PageQuery {
    pub fn validate(&self) -> Result<(), QueryValidationError> {
        if self.page == 0 || !(1..=200).contains(&self.page_size) {
            return Err(QueryValidationError::InvalidPagination);
        }
        Ok(())
    }

    pub fn limit(&self) -> i64 {
        self.page_size as i64
    }

    pub fn offset(&self) -> i64 {
        (self.page - 1) as i64 * self.limit()
    }
}

#[derive(Debug, serde::Serialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub page: u32,
    pub page_size: u32,
    pub total: i64,
}
```

Default `page_size=50`, valid range `1..=200`, `page>=1`. Cursor/page token tối đa 512 bytes;
single filter value và search text tối đa 256 UTF-8 bytes; tối đa 4 sort fields và 16 typed
filter predicates. Chỉ typed allowlisted filter/sort field được nhận, không raw SQL column/order
expression. Offset pagination V1 không có offset client-supplied unbounded; history lớn sau này
ưu tiên cursor pagination.

Mọi list endpoint reuse type này.

## 11.2. Common API response

```rust
#[derive(serde::Serialize)]
pub struct ApiResponse<T> {
    pub data: T,
}
```

List:

```rust
ApiResponse<Page<AgentDto>>
ApiResponse<Page<DeviceDto>>
ApiResponse<Page<HistoryMessageDto>>
```

Không tạo response format khác nhau cho từng resource.

## 11.3. Common API error

Tạo:

```text
src/api/error.rs
```

```rust
pub enum ApiError {
    BadRequest(String),
    PayloadTooLarge,
    UnsupportedMediaType(String),
    NotFound(String),
    Conflict(String),
    Database(DatabaseError),
    Internal,
}
```

Implement một lần:

```rust
impl IntoResponse for ApiError { ... }
```

Repository errors map một lần sang:

```text
404
409
400
503 database_busy          (SQLite BUSY/LOCKED sau busy_timeout)
503 database_unavailable   (pool timeout/I/O/storage)
500
```

Handler mới không được tự map `sqlx::Error`.

Admin error response dùng envelope không chứa raw internal error:

```json
{
  "error": {
    "code": "revision_conflict",
    "request_id": "<server UUID>"
  }
}
```

## 11.4. Whitelisted sorting

Không cho client gửi raw SQL sort column.

Sai:

```text
?sort=<raw string inserted into SQL>
```

Đúng:

```rust
pub enum AgentSort {
    Name,
    CreatedAt,
    UpdatedAt,
}
```

```rust
impl AgentSort {
    fn sql_column(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::CreatedAt => "created_at",
            Self::UpdatedAt => "updated_at",
        }
    }
}
```

Sort direction cũng dùng enum `Asc/Desc`.

## 11.5. Typed filters

Mỗi resource có typed query:

```rust
#[derive(Debug, Deserialize)]
pub struct AgentListQuery {
    #[serde(flatten)]
    pub page: PageQuery,
    pub enabled: Option<bool>,
    pub search: Option<String>,
}
```

```rust
#[derive(Debug, Deserialize)]
pub struct HistoryListQuery {
    #[serde(flatten)]
    pub page: PageQuery,
    pub session_id: Option<String>,
    pub device_id: Option<i64>,
    pub agent_id: Option<i64>,
    pub template_id: Option<i64>,
    pub role: Option<HistoryRole>,
}
```

Dynamic SQL dùng `sqlx::QueryBuilder`, nhưng chỉ append clauses từ typed/validated filters.

## 11.6. Common query helpers

Tạo helpers cho các phần thật sự generic:

```rust
pub struct QueryLimits {
    pub limit: i64,
    pub offset: i64,
}

pub fn normalize_search(value: Option<String>) -> Option<String>;
pub fn unix_millis_now() -> i64;
```

Không cố tạo `GenericRepository<T>` quá trừu tượng cho mọi table. SQLx không phải ORM và domain relations của các bảng khác nhau.

Mục tiêu reuse là:

```text
pool
transaction
pagination
sort direction
API response
API error
filter conventions
timestamp
validation
```

SQL query đặc thù domain vẫn nằm trong repository tương ứng.

---

# 12. API module sẵn sàng cho CRUD/query sau này

Tạo ngay module API, dù phase đầu chỉ expose một số endpoint.

```text
src/api/
├── mod.rs
├── error.rs
├── response.rs
├── agents.rs
├── templates.rs
├── providers.rs
├── mcp.rs
├── devices.rs
└── history.rs
```

`api/mod.rs`:

```rust
pub fn router() -> Router<AppState> {
    Router::new()
        .nest("/agents", agents::router())
        .nest("/templates", templates::router())
        .nest("/providers", providers::router())
        .nest("/mcp", mcp::router())
        .nest("/devices", devices::router())
        .nest("/history", history::router())
}
```

Main app:

```rust
Router::new()
    .route("/health", get(health))
    .route("/ready", get(ready))
    .route("/voice/ota/", ...)
    .route("/voice/v1/", get(websocket::handler))
    .nest("/api/admin", api::router())
    .with_state(app_state)
```

## 12.1. Pattern khi thêm một endpoint query mới

Ví dụ cần:

```text
GET /api/admin/devices/:id/history?session_id=...
```

Developer chỉ cần:

1. thêm typed query DTO;
2. gọi method repository/service có sẵn;
3. trả `ApiResponse<T>`.

Handler:

```rust
async fn list_device_history(
    State(state): State<AppState>,
    Path(device_id): Path<i64>,
    Query(query): Query<HistoryListQuery>,
) -> Result<Json<ApiResponse<Page<HistoryMessageDto>>>, ApiError> {
    let page = state
        .services
        .history
        .list_for_device(device_id, query)
        .await?;

    Ok(Json(ApiResponse { data: page }))
}
```

Không có:

- pool creation;
- PRAGMA;
- raw `sqlx::Error` mapping;
- custom pagination implementation;
- custom response envelope;
- duplicated transaction code.

Đó là ý nghĩa của yêu cầu “thêm API query DB có sẵn, không viết lại”.

---

# 13. Application state

Branch `dev-test` hiện có `AppState` chứa provider/runtime catalogs. Bổ sung DB và service container, không thay ownership runtime hiện tại.

Đề xuất:

```rust
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,

    pub providers: Arc<ProviderCatalog>,
    pub runtimes: Arc<RuntimeCatalog>,

    pub database: Arc<Database>,
    pub repositories: Arc<Repositories>,
    pub services: Arc<Services>,
    pub secret_resolver: Arc<dyn SecretResolver>,

    pub worker_supervisor: Arc<WorkerSupervisor>,
    pub active_turn_limiter: Arc<ActiveTurnLimiter>,
}
```

Container:

```rust
pub struct Repositories {
    pub agents: AgentRepository,
    pub templates: TemplateRepository,
    pub providers: ProviderRepository,
    pub mcp: McpRepository,
    pub devices: DeviceRepository,
    pub history: HistoryRepository,
    pub audit: AuditRepository,
}
```

```rust
pub struct Services {
    pub session_config: SessionConfigService,
    pub templates: TemplateService,
    pub history: HistoryService,
    pub audit: AuditService,
}
```

Không để handler tự construct repository ở mỗi request.

---

# 14. Async application startup

Hiện `application(config)` là synchronous. Database connection/migration là async, nên nên chuyển startup seam thành async thay vì dùng `block_on` bên trong.

Đề xuất:

```rust
pub async fn application(config: AppConfig) -> Result<Router, AppStartError> {
    let database = Database::connect(&config.database).await?;
    let secret_resolver: Arc<dyn SecretResolver> = Arc::new(EnvSecretResolver::new());

    let repositories = Repositories::new(database.clone());

    let loaded = load_provider_catalog(&config, &repositories.providers, &secret_resolver).await?;

    let state = AppState::new(
        config,
        loaded,
        database,
        repositories,
        secret_resolver,
    );

    Ok(router(state))
}
```

Khi `database.enabled=true`, startup theo thứ tự validate config → open SQLite → PRAGMA
→ read SQLx migration state/compatibility → forward migrations → repositories/services →
application → bind listener. `migrate_on_start=false` chỉ cho boot khi schema đã current;
schema cũ không được silently run. Lỗi config/path/open/PRAGMA/migration/schema compatibility
fail-fast trước bind; không warn rồi chạy
không DB. Khi false, không mở SQLite và Voice giữ flow hiện tại. Runtime DB failure sau
boot không shutdown process: new DB-admitted session và admin DB query trả `503`,
HistoryWriter drop best-effort, session đã admit tiếp tục snapshot cũ. `/health` là
liveness process. `/ready` trả `200` chỉ khi process có thể nhận connection mới: startup complete,
schema compatible, required RuntimeCatalog entries healthy/present, admission resolver operational
và DB reachable nếu feature active cần DB. `/ready` không full admission, không resolve Device,
không initialize/tools/list External MCP, không reload model/secret. Optional External MCP fail-soft
không làm non-ready; runtime DB failure làm `/ready` non-200 khi database-backed admission active,
nhưng session đã admit không bị đổi.

Shutdown: khi nhận shutdown, stop listener/admission/request mới trước và cấm start tool call/DB
admission mới. Session đang mở được drain tối đa `shutdown.grace_ms`, rồi controlled-close; không
cho shutdown kéo dài vô hạn. HistoryWriter chỉ best-effort flush trong cùng deadline, không kéo dài
grace period. External MCP/DB cleanup follow cancellation, không đổi prior session semantics.

Test seam vẫn có thể có:

```rust
pub fn router_with_state(state: AppState) -> Router
```

Không để production startup và tests phụ thuộc cứng vào cùng constructor quá lớn.

---

# 15. Provider DB integration với `dev-test`

Branch `dev-test` đã có:

```text
provider_defaults
ProvidersConfig instances
ProviderCatalog
RuntimeCatalog
EffectiveProviderBindings
```

Không bỏ các abstraction này.

Thay vào đó mở rộng source của provider definitions.

## 15.1. Bootstrap model

```text
config.toml providers
        +
SQLite providers
        ↓
Validated ProviderDefinitionCatalog
        ↓
compiled_provider_registry
        ↓
ProviderCatalog
RuntimeCatalog
```

Provider `key` trong DB phải nằm chung namespace với config provider instance IDs.

Collision rule:

```text
DB provider key == TOML provider key
    → startup validation error
```

Không silently override.

## 15.2. Server defaults luôn phải load

Các provider được `provider_defaults` tham chiếu phải load kể cả không có DB Agent/Template dùng chúng.

## 15.3. Provider load plan: required và optional

```text
RequiredRuntimeProviders =
  ServerProviderDefaults
  UNION ProvidersReferencedByEnabledAgentDefaultTemplate

OptionalRuntimeProviders =
  ProvidersReferencedOnlyByEnabledNonDefaultTemplateAssignment
```

`ProviderLoadPlan { required, optional }` dedupe union để một provider không load hai
lần. Required failure fail-fast trước listener. Optional provider vẫn attempt load: load
success vào RuntimeCatalog, còn model/adapter/secret-resolution/runtime failure giữ
`runtime_available=false` và chỉ exclude candidate non-default kèm metric/warning.
Provider unbound/orphan không thuộc set: skip load, `runtime_available=false`/
`not_loaded`, không block boot. `enabled` nghĩa là được phép bind, không có nghĩa runtime
phải materialize.

Default Template phải structurally valid trước: assignment/binding đủ, Provider enabled
và config reference hợp lệ; runtime validity là bước materialize kế tiếp. Default phải
pass cả hai, còn non-default structurally/runtime invalid fail-soft theo candidate.

Startup:

```text
DB migrate + resolve ProviderLoadPlan
  ↓
load required (failure → startup fail)
  ↓
attempt optional (failure → status unavailable + continue)
  ↓
build ProviderCatalog + RuntimeCatalog
```

Provider required resolve secret/load fail (model/adapter/secret/runtime build) fail-fast
trước listener. Runtime giữ credential snapshot cùng process lifetime; secret deployment
rotate chỉ có hiệu lực sau restart và RuntimeCatalog mới. Provider optional resolve secret/load
fail giữ `runtime_status=unavailable` và exclude dependent non-default Template; unbound không
resolve secret lúc startup. Admin GET chỉ trả
coarse `runtime_status` (`not_loaded`, `unavailable`, `loaded`) và
`runtime_available`, không raw error. Admin bind provider chưa load được persist với
`requires_restart=true`; default profile dùng nó trả `503`, non-default candidate bị
exclude. Không load ONNX/model lần đầu khi Device connect.

Provider loader phải gọi cùng `ProviderConfigValidator` trước secret resolution/runtime build:
raw byte cap → JSON parse → depth/node cap → recursive protected-key guard → deserialize discriminator-specific typed config với
`#[serde(deny_unknown_fields)]` → adapter-specific validation. Required row invalid fail
startup; optional invalid giữ unavailable và exclude candidate. Không tin chỉ Admin API đã
validate vì DB có thể được migrate, restore hoặc sửa trực tiếp.

Startup validation pass áp lên mọi Provider enabled, kể cả provider unbound. Unbound vẫn skip
secret resolution/runtime build và không block boot, nhưng config invalid có `runtime_status`
coarse `unavailable` khi Admin inspect; chỉ required Provider invalid mới fail startup.

## 15.4. Runtime resolution

Template DB binding trả provider keys:

```rust
EffectiveProviderBindings {
    vad: "silero_default",
    asr: "gipformer_vi",
    llm: "openai_primary",
    tts: "zerotts_maichi",
    vision: None,
}
```

Session dùng existing `RuntimeCatalog`:

```text
runtimes.vad(key)
runtimes.asr(key)
runtimes.llm(key)
runtimes.tts(key)
```

Không tạo provider runtime mới per session.

---

# 16. Provider mutation qua API

Có hai loại DB change:

## 16.1. Metadata-only/config không ảnh hưởng loaded runtime

Ví dụ:

```text
provider.name
provider.enabled cho provider chưa active
```

có thể commit DB trực tiếp.

## 16.2. Runtime-affecting change

Ví dụ:

```text
adapter
model
base_url
voice
num_threads
```

Không được update DB rồi để current RuntimeCatalog chạy config cũ mà không báo gì.

Phase đầu nên chọn contract đơn giản:

```text
runtime-affecting provider change
  → persist DB
  → mark requires_restart = true ở API response/service result
  → provider catalog mới có hiệu lực sau server restart
```

Không implement hot reload provider model trong cùng phase database.

Response của provider CRUD phải phân biệt rõ Database Desired Configuration với
runtime process hiện tại, tối thiểu bằng `requires_restart`; có thể trả thêm
`runtime_available` hoặc `runtime_status`. DB không được biểu diễn provider vừa
update là đã effective trước restart.

`RuntimeStatus` là `not_loaded | unavailable | loaded`, trả lời runtime usable có tồn
tại trong process không. `runtime_matches_desired` trả lời loaded runtime có được build
từ desired revision hiện tại không; metadata nội bộ giữ `LoadedProviderRuntimeMeta {
provider_id, loaded_revision }`. `loaded` không đồng nghĩa newest: PATCH runtime-affecting
có thể trả `loaded`, `runtime_matches_desired=false`, `requires_restart=true`. Restart
thành công làm loaded revision match và xóa restart requirement. Provider unbound chưa
load: `not_loaded`, `runtime_matches_desired=false`, `requires_restart=false`; binding
nó vào Template cần dùng mới khiến `requires_restart=true` ở effective usage/binding level.

Provider config parsing dùng discriminator, không để adapter tự lấy field tùy ý từ
`serde_json::Value`:

```rust
enum ProviderConfig {
    OpenAi(OpenAiProviderConfig),
    ZeroTts(ZeroTtsProviderConfig),
    Gipformer(GipformerProviderConfig),
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct OpenAiProviderConfig {
    base_url: Url,
    model: String,
    timeout_ms: u64,
}
```

Không typed config nào chứa `api_key`, `token`, `password`, `secret`, arbitrary `Value`,
`HashMap<String, Value>` hoặc arbitrary headers/options escape hatch. Nếu adapter tương lai
thực sự cần nested extensibility, nó cần schema/allowlist, protected-key scan và size/depth
bounds riêng trước khi được thêm.

`ProviderConfigValidator` là pure/shared seam cho Admin API và startup DB loader:

```rust
const MAX_PROVIDER_CONFIG_BYTES: usize = 64 * 1024;
const MAX_PROVIDER_CONFIG_DEPTH: usize = 16;
const MAX_PROVIDER_CONFIG_NODES: usize = 512;

fn validate_provider_config_shape(
    raw: &[u8],
    value: &serde_json::Value,
) -> Result<(), ProviderConfigValidationError>;
```

`nodes` là tổng mọi object key và array item toàn cây JSON; walker tăng depth trước khi
descend và short-circuit ngay khi vượt cap. Admin mutation pipeline là: raw request/body
bound → config UTF-8 byte cap → JSON parse → depth/node cap → recursive protected-key guard →
typed deserialize theo adapter + `deny_unknown_fields` → adapter validation → canonical
serialize → persist. Generic cap chạy trước typed deserialization để tránh allocation/xử lý
structure quá lớn.
Guard canonicalize **key để so sánh duy nhất** bằng ASCII lowercase và bỏ `_`, `-`, whitespace;
không sửa JSON được persist. Exact denylist là `apikey`, `token`, `accesstoken`,
`refreshtoken`, `bearertoken`, `password`, `passwd`, `secret`, `clientsecret`,
`authorization`, `proxyauthorization`, `credential`, `credentials`, `privatekey`, `secretref`.
Không substring-match (nên `max_tokens`, `tokenizer`, `token_budget` hợp lệ) và không scan
heuristic value như `sk-`/`Bearer`/JWT. Scan toàn bộ object và arrays; generic cap, invalid
JSON, protected field hoặc typed validation failure đều trả client `400 provider_config_invalid`.
Telemetry nội bộ chỉ dùng bounded reason `too_large|too_deep|too_many_nodes|invalid_json|
protected_field|typed_validation_failed`; không echo path/key/value/config body vào response
hay log. Generic cap là resource-abuse boundary; typed schema vẫn phải giới hạn semantic field
nhỏ hơn (ví dụ model 256 bytes, base URL 2048 bytes, voice 128 bytes, custom list 32 items).

Template binding switch giữa **các provider runtime đã load** có thể có hiệu lực per session/next turn mà không restart.

---

# 17. Session config resolver

Tạo service trung tâm:

```text
src/database/service/template_resolver.rs
```

Đây là nơi duy nhất resolve:

```text
Device → Agent → Template → Providers + MCP
```

Contract:

```rust
pub struct EffectiveSessionProfile {
    pub device: DeviceSnapshot,
    pub agent: AgentSnapshot,
    pub template: Option<TemplateSnapshot>,
    pub providers: EffectiveProviderBindings,
    pub system_prompt: Arc<str>,
    pub language: Arc<str>,
    pub mcp: Arc<[ResolvedMcpBinding]>,
    pub template_catalog: Arc<TemplateSwitchCatalog>,
    pub revision: SessionProfileRevision,
}

pub struct TemplateSwitchCatalog {
    entries: HashMap<String, ResolvedTemplateProfile>,
}

pub struct ResolvedTemplateProfile {
    pub template_id: i64,
    pub template_key: String,
    pub language: Arc<str>,
    pub system_prompt: Arc<str>,
    pub providers: EffectiveProviderBindings,
    pub runtimes: ResolvedSessionRuntimes,
}
```

Method:

```rust
pub async fn resolve_for_device(
    &self,
    protocol_device_id: &str,
) -> Result<EffectiveSessionProfile, SessionConfigError>;
```

Pseudo-flow:

```text
SELECT device by protocol device_id
  ↓
device enabled?
  ↓
SELECT agent
  ↓
agent enabled?
  ↓
load all template assignments
  │
  ├─ no assignment exists
  │    → use config.effective_agent + provider_defaults
  │    → TemplateSwitchCatalog {} and no server.switch_template tool
  │
  └─ one or more assignments exist
       → require exactly one enabled default assignment
       → default candidate must validate Template, bindings and Loaded Runtime
       → validate every non-default enabled assignment independently
       → build TemplateSwitchCatalog from valid candidates only
  ↓
load Agent MCP bindings
  ↓
return immutable snapshot
```

WebSocket/session code không thực hiện các query này từng cái một.

An Agent chỉ fallback server defaults khi không có **bất kỳ** Template assignment
nào. Assignment tồn tại nhưng thiếu/disabled default, hoặc default trỏ
Template/provider/runtime invalid là `SessionConfigError::AgentRuntimeUnavailable`:
reject `503 Agent runtime unavailable` trước upgrade. Non-default assignment enabled
nhưng invalid chỉ bị exclude khỏi `TemplateSwitchCatalog`, emit structured warning/
metric và không được expose cho switch; nó không làm Agent unavailable. Internal logs
dùng error kind ổn định nhưng không log `config_json`, endpoint credential hoặc secret
reference.

Sau admission, disable Device/Agent/Template, đổi prompt/language, Provider binding/
revision hoặc Agent MCP binding từ Admin API không đổi snapshot hiện hành, kể cả ở
turn kế tiếp. Chỉ `server.switch_template` do chính SessionActor xử lý mới thay
profile từ next-turn boundary, bằng lookup `TemplateSwitchCatalog`. ADR-0052 ghi
decision này.

---

# 18. WebSocket integration

## 18.1. Identity phải lấy trước upgrade/session initialization

WebSocket HTTP handler hiện đã nhận `device-id`/`client-id` từ header/query.

Giữ identity value lại và resolve DB trước khi build session runtime snapshot.

Flow:

```text
HTTP WS request
  ↓
validate protocol/device/client/auth
  ↓
SessionConfigService.resolve_for_device(device-id)
  ↓
resolve RuntimeCatalog handles
  ↓
upgrade
  ↓
ClientHello
  ↓
create SessionActor(session_id=UUID)
```

Khi `database.enabled = true` và `database.devices.admission_enabled = true` ở DB-4,
DB/resolver là dependency bắt buộc cho connection mới. Policy V1 là:

```text
unknown hoặc disabled device
  → reject 403 before WS upgrade

invalid Agent profile hoặc DB/pool/resolver unavailable
  → reject 503 before WS upgrade
```

Không fallback server defaults sau DB timeout/resolver failure. Trước DB-4 hoặc khi
`admission_enabled = false`, Voice giữ flow hiện tại không DB admission. Session đã
admit tiếp tục bằng immutable snapshot nếu SQLite chết sau đó. Internal diagnostics
dùng `database_busy`, `database_unavailable`, `database_pool_timeout`, `device_resolution_failed` hoặc
`agent_profile_resolution_failed`; client chỉ nhận coarse-grained message.

Nếu bật explicit auto-register:

```toml
[database.devices]
auto_register = true
auto_register_agent_key = "dev-bootstrap-agent"
```

thì chỉ dùng cho dev/migration. `auto_register_agent_key` bắt buộc non-empty và
phải resolve đến đúng một Agent enabled; thiếu key, Agent missing/disabled hoặc lỗi
write đều reject `403` trước upgrade, không auto-register một phần. Device mới tạo
`enabled=true`, bind ngay Agent đó, và có metadata `{source: "auto_register",
registered_at: ...}` cùng timestamps; không lưu authorization header, bearer token,
query auth hoặc ClientHello raw. Dùng UNIQUE(`device_id`) cùng transaction/upsert-safe
logic để race hai connection không tạo duplicate. Không auto-register ngầm.
ADR-0049 ghi decision này.

## 18.2. Session snapshot

Khi actor được tạo:

```rust
SessionActor {
    session_id,
    device_db_id,
    agent_id,
    active_template_id,
    // ...
}
```

Provider runtime handles và prompt snapshot được resolve một lần.

Không query DB trên mỗi Opus frame.

`SessionActor` nhận `EffectiveSessionProfile` đã materialize và không giữ row/repository
để live-reconfigure. Revoke realtime, nếu cần, đi qua `SessionRegistry →
RevokeSession { reason } → SessionActor controlled shutdown` trong feature riêng.

---

# 19. Template switch tool

Tạo server internal tool, ví dụ:

```text
server.switch_template
```

Input schema:

```json
{
  "type": "object",
  "properties": {
    "template": {
      "type": "string"
    }
  },
  "required": ["template"],
  "additionalProperties": false
}
```

## 19.1. Validation

Khi tool được gọi:

```text
lookup template_key trong SessionActor.template_catalog
  ↓
candidate đã validate/resolve lúc admission?
  ↓
set pending_template
```

Không query SQLite, không update `agents`/`devices`, và không tạo runtime từ DB. Unknown
template hoặc candidate excluded từ catalog là switch fail, không tăng revision.

## 19.2. Commit switch boundary

Chỉ apply pending template sau terminal boundary của current turn.

```rust
fn apply_pending_template_after_turn_close(&mut self) {
    if let Some(next) = self.pending_template.take() {
        self.active_profile = next.profile;
        self.profile_revision += 1;
    }
}
```

Khi apply phải atomically replace:

- prompt/session prompt state;
- language;
- VAD config/runtime handle;
- ASR runtime handle;
- LLM runtime handle;
- TTS runtime handle.

Nếu VAD capture cycle đang active, switch chỉ apply khi current turn/cycle ở safe boundary; không thay VAD session giữa frame sequence.

`SessionProfileRevision` là counter `u64` session-local: start `1`, chỉ tăng atomically
khi pending profile được apply sau normal turn boundary. Nó không phải
`template.revision`, `provider.revision` hay DB row version; switch fail hoặc turn bị
abort trước boundary không tăng. Dùng cho logging, stale-event rejection, debug và
switch telemetry; không persist vào `history_messages`.

---

# 20. MCP Streamable HTTP integration

Không trộn hai khái niệm:

```text
Device MCP
  Client/ESP32 ↔ Rust qua Voice WebSocket

External MCP
  Rust ↔ MCP Server qua Streamable HTTP
```

Tạo module khác:

```text
src/tools/external_mcp/
├── mod.rs
├── client.rs
├── manager.rs
├── registry.rs
└── transport.rs
```

DB chỉ cung cấp MCP config snapshot.

`ExternalMcpManager` application-owned có thể reuse connection/client state giữa sessions khi transport/library cho phép.

Agent session nhận filtered tool snapshot từ các MCP servers bind với Agent.

Không query `mcp_servers` mỗi tool call.

LLM-visible namespace cố định:

```text
Device MCP    → self.<tool>
External MCP  → external.<server_key>.<tool>
```

Registry map `llm_visible_name → ToolOrigin → server instance + original_name`; tên
sanitized/namespaced không là authority để route trực tiếp. `ToolOrigin` tách
`Device { original_name }` và `ExternalMcp { server_key, original_name }`.

`normalize_external_tool_segment` là pure function có test vector cố định. Mỗi input
server key hoặc original MCP tool name là đúng **một** segment (không diễn giải
separator vendor-specific): lowercase Unicode; mỗi run ký tự ngoài ASCII
`[a-z0-9_]` thành `_`; collapse rồi trim `_`; empty là invalid; prefix `x_` nếu ký tự
đầu không phải `[a-z]`; và reject nếu dài quá 64, không truncate. Vì vậy
`Home-Assistant` + `Light/Turn-On` thành
`external.home_assistant.light_turn_on`; `foo-bar` và `foo_bar` collision thì reject,
không hash suffix. Test tối thiểu: `ABC → abc`, `foo-bar → foo_bar`,
`123tool → x_123tool`, `__tool → tool`, Unicode-only/empty/over-64 invalid và collision
pair reject.

`validate_external_tool_schema(&serde_json::Value)` chỉ nhận bounded supported subset
JSON Schema trước LLM conversion. Top-level phải `type: object`; allowlist field là
`type`, `description`, `properties`, `required`, `additionalProperties`, `enum`,
`const`, `items`, `minimum`, `maximum`, `minLength`, `maxLength`. Reject `$ref`,
`$defs`/`definitions`, remote URI, recursive schema, `allOf`/`anyOf`/`oneOf`, `not`,
`if`/`then`/`else`, `unevaluatedProperties`, `dynamicRef`, `recursiveRef` và mọi shape
không allowlist. `properties` phải object, `items` là schema hợp lệ, required chỉ chứa
property tồn tại và type thuộc primitive/object/array supported. Bound: depth `12`, nodes
`512`, properties total `256`, properties/object `64`, required `64`, enum items `128`.
Không truncate/rewrite; một tool schema invalid reject toàn bộ External MCP server catalog.

Validation order: raw byte cap → JSON parse → structural caps → supported-schema
validation → tool-name normalize/collision → LLM tool conversion.

V1 External MCP fail-soft: server unavailable, initialize/tools-list timeout, invalid
tools list hoặc tool-name collision chỉ loại tools của server đó khỏi snapshot; Voice
Session vẫn được accept. Không advertise stale cached tools như đã verify trong
session hiện tại. MCP chết sau session start không đóng Voice Session; tool call liên
quan fail controlled. Tool name không sanitize hợp lệ bị bỏ; nếu server không còn tool
hợp lệ thì coi unavailable. Collision nội bộ cùng server reject cả server; collision
giữa server sau namespace bị chặn bởi UNIQUE(server key) và config validation; collision
với Device MCP không thể theo namespace. Không có first/last/bind-order wins.
Diagnostics dùng bounded kind `mcp_server_unavailable`,
`mcp_initialize_failed`, `mcp_tools_list_timeout`, `mcp_tools_list_invalid` hoặc
`mcp_tool_name_invalid`, `mcp_tool_name_collision`, `mcp_server_catalog_rejected`;
không log auth headers, payload nhạy cảm hay tool result.
ADR-0053 ghi availability contract này.

`headers_json` parse thành canonical `HeaderName` rồi reject case-insensitive protected
headers: `authorization`, `proxy-authorization`, `cookie`, `set-cookie`, `host`,
`content-length`, `transfer-encoding`, `connection`, `upgrade`, `te`, `trailer`,
`proxy-authenticate`, `www-authenticate`, `keep-alive`. Request assembly cố định:
validated static headers → standard MCP headers → typed ExternalMcpAuth injection →
send. Không expose generic `insert_header()` sau auth injection và không merge map theo
last-write-wins; typed auth là credential path duy nhất.

HTTPS External MCP luôn dùng normal certificate chain và hostname validation; không có
`danger_accept_invalid_certs`, insecure TLS switch hoặc custom bypass trong V1. HTTP chỉ có thể
qua `mcp.external.network` LAN allowlist đã chốt; redirects vẫn disabled.

Loopback (`127.0.0.0/8` và `::1/128`) nằm trong LAN scope của `allow_http_lan`, vì MCP server homelab
thường chạy cùng host. Nó **không** phải bypass: HTTP loopback chỉ hợp lệ khi `allow_http_lan=true`
**và** operator đã ghi rõ destination vào allowlist — IP literal phải match `allowed_cidrs`,
hostname phải match `allowed_hosts`, và cùng destination đó phải vượt validation sau DNS
resolution. Allowlist rỗng thì loopback cũng bị từ chối, và không có test-only bypass. Host phải
so khớp bằng typed host (`url::Host`), không phải `Url::host_str`, vì `host_str` render IPv6
literal dạng `[::1]`. ADR-0056 là nguồn chốt.

Mỗi WS admission lấy fresh External MCP snapshot: `initialize → tools/list` tới hết
pagination → normalize/validate → Session MCP snapshot. Có thể reuse reqwest client,
connection pool, TLS session, DNS cache và HTTP transport; không reuse tools/list,
sanitized registry hay availability state cũ làm authoritative. Resolve các binding
song song bounded, timeout tối đa
`mcp.external.per_server_resolution_timeout_ms` cho mỗi server và toàn bộ admission
không vượt `mcp.external.overall_resolution_budget_ms`. Optional MCP fail bị exclude;
semantics `required=true → 503` chỉ có hiệu lực ở phase hỗ trợ required sau V1.

Áp caps tại raw/untrusted boundary trước LLM schema conversion, không truncate: một
server vượt `max_tools_per_server`, `max_pages_per_server`, schema/description bytes
hoặc pagination cap thì reject toàn bộ server catalog. Chỉ sau khi mọi server pass mới
áp `max_tools_per_session`; nếu aggregate vượt cap, emit
`external_mcp_session_tool_cap_exceeded`, exclude **toàn bộ** External MCP snapshot
và accept Voice session. Không lấy partial catalog hoặc loại “server cuối” theo bind
order; Device MCP không bị ảnh hưởng. Required semantics trong phase sau sẽ biến cap
failure của required MCP thành `503`.

Mỗi `ResolvedExternalMcp` trong Effective Session Profile snapshot server binding,
original tool mapping và `call_timeout`; admin đổi timeout/binding không đổi session cũ.
`SessionActor` không giữ raw `SecretValue`; ownership credential nằm trong immutable client
handle của snapshot:

```rust
pub struct ResolvedExternalMcp {
    pub server_key: String,
    pub client: Arc<ExternalMcpClient>,
    pub tools: Arc<[ResolvedExternalTool]>,
    pub call_timeout: Duration,
}
```

`ExternalMcpClient` sở hữu base URL, validated static headers, typed auth, `SecretValue` và
HTTP client. Admission resolve credential đúng một lần trước `initialize/tools/list`; mọi
`tools/call` sau dùng client snapshot đó, không re-resolve hot path. Secret rotate chỉ hiện
ra với session admission sau; session đang mở giữ credential cũ đến disconnect.
Session Tool Catalog được tạo từ discovery đã validate tại admission và immutable đến
disconnect. Một `tools/call` timeout/unavailable/invalid response chỉ là telemetry của
invocation: không remove tool/server khỏi catalog, không disable DB/runtime availability,
và không có circuit breaker implicit trong V1. Invocation sau vẫn dùng tool snapshot cũ,
với one-attempt, bounded timeout và turn budget như mọi call khác. Circuit breaker nếu
cần là feature riêng có state `Closed`/`Open`/`HalfOpen`, cooldown và recovery probe rõ
ràng.

`ExternalMcpCallLimiter` giữ semaphore global theo immutable MCP server identity, shared
giữa mọi Voice Session trong process; default có 16 permits/server. Executor acquire một
permit trước outbound `tools/call`, chỉ giữ trong đúng lifetime của request và release trước
LLM continuation. Thời gian chờ acquire tính vào turn execution budget; nếu budget hết
trước khi có permit, không gửi request và terminalize call bằng
`external_tool_unavailable`. Không có queue/retry riêng ngoài budget, không giữ permit qua
ToolResult/TTS, và overload của một server không ảnh hưởng server khác hay Device MCP.
Metrics chỉ dùng bounded server key/outcome, ví dụ `external_mcp_call_limiter_rejected_total`.

Secret cho External MCP chỉ resolve khi chuẩn bị admission snapshot, ngay trước
`initialize/tools/list`, không khi đọc DB row. Resolve fail loại server optional khỏi snapshot
theo fail-soft và emit `external_mcp_secret_resolution_failed` với bounded reason
`secret_missing|secret_resolver_unavailable|secret_invalid`; không log `secret_ref`, value,
Authorization hay resolved header. Khi `required=true` được hỗ trợ sau này, cùng failure sẽ
trở thành `503` trước upgrade.
Khi LLM gọi External MCP, resolve `llm_visible_name → ToolOrigin::ExternalMcp →`
snapshot binding + original name rồi thực hiện đúng một bounded `tools/call` attempt.
Application layer và SessionActor đều không retry call, vì remote có thể đã thực hiện
side effect trước khi response timeout. Chỉ protocol/idempotency mechanism được thiết kế
riêng sau này mới có thể thay rule này.

Mỗi tool call luôn terminalize thành ToolResult để continuation không dangling:
response hợp lệ → normal result; timeout → `{ "error": "external_tool_timeout" }`;
connection/server unavailable → `{ "error": "external_tool_unavailable" }`;
remote authentication reject (`401`/`403`) → `{ "error": "external_tool_auth_failed" }`;
malformed/unsupported response → `{ "error": "external_tool_invalid_response" }`;
protocol fault dùng `external_tool_protocol_error`. Synthetic payload server-generated
không chứa remote body, exception, URL, arguments, secret hay internal diagnostic.
Failure không close Voice Session và continuation vẫn nhận AssistantToolCall/ToolResult
cặp hoàn chỉnh.

`external_tool_auth_failed` không trigger secret refresh, retry, catalog mutation hay DB/runtime
availability mutation. Session mới có thể resolve credential đã rotate khi admission; session
cũ chỉ tiếp tục one-attempt policy với snapshot cũ.

Tool-round executor chung cho `self.*` và `external.*` chạy strict sequential theo model
order, không `join_all`, FuturesUnordered hay spawn parallel. Với mọi tool call, executor
check cancellation rồi execute một lần, append terminal result đúng index và mới bắt đầu
call tiếp theo. Failure một call không dừng round; chỉ abort/disconnect/generation
cancellation/shutdown dừng phần còn lại và cấm bắt đầu outbound side-effecting call mới.
Invariant: `calls.len() == results.len()` và `results[i]` đúng `calls[i]`; model order =
execution order = ToolResult/history/continuation order.

Trước call đầu tiên của round, validate toàn bộ `calls.len() <= max_calls_per_round`; vi
phạm là `tool_call_limit_exceeded`, execute zero call trong round và terminal turn failure.
Trước round mới, check `max_rounds_per_turn`; vượt là `tool_round_limit_exceeded` trước
bất kỳ call nào. Turn execution budget bắt đầu lúc tool call đầu tiên; mỗi call dùng
`min(call_timeout, remaining_turn_budget)`. Hết budget trước call mới là
`tool_execution_budget_exceeded`; hết khi in-flight thì cancel/drop future, không retry
và không continuation. Cap/budget/cancellation là turn-level terminal failure, không
synthetic ToolResult/sentinel vì không có completed matching ToolCall.

Cancellation là TurnId + GenerationId boundary. Abort/disconnect/invalidation mark turn
cancelled, cancel/drop in-flight future khi transport hỗ trợ, cấm start call sau và check
gate trước commit result vào completed round. Late response không active bị discard:
không ToolResult, continuation, SQLite history hay TTS. Completed prefix trước cancel vẫn
giữ semantics history hiện có. Metrics bounded gồm
`external_tool_call_cancelled_total` và `external_tool_late_response_discarded_total`;
không arguments/result.

External MCP success result cũng untrusted. Chỉ accept Text hoặc Structured JSON; reject
binary/blob, image, audio, embedded resource, resource link, arbitrary MIME, stream,
unknown content type hay mix result có bất kỳ content unsupported nào. Text items join
theo remote order bằng `\n`, Structured JSON parse/validate rồi `serde_json::to_string`;
không debug stringify. Canonical payload cuối cùng phải UTF-8 và
`<= mcp.external.limits.max_external_tool_result_bytes` (16 KiB default); không truncate,
summarize hay partial accept. Pipeline: protocol validation → terminal success → content
allowlist → extract complete semantic result → canonical serialize → byte cap. Bất kỳ
failure nào là `external_tool_invalid_response`; result valid vào current tool round nhưng
không persist vào `history_messages` và không log body.

---

# 21. History persistence

Chỉ implement/khởi động HistoryWriter khi `database.history.enabled = true` (DB-8).
Khi false, không tạo `HistoryWrite`, không enqueue và không write transcript.

## 21.1. Session ID

Dùng cùng `session_id` mà server tạo cho Voice Session:

```rust
Uuid::new_v4().to_string()
```

Không tạo một database-session identity thứ hai.

## 21.2. Sequence

`SessionActor` giữ:

```rust
history_sequence: u64
```

Tăng khi persist mỗi message.

Hoặc `HistoryService` cấp sequence transactionally.

Với một SessionActor là single owner, ưu tiên actor-side monotonic counter để không cần `MAX(sequence)` query trên hot path.

## 21.3. User message

Sau ASR final non-empty đã được accepted cho current turn:

```text
commit DialogueHistory user
  +
enqueue async DB HistoryWrite(user)
```

DB write không được block audio actor lâu.

Tạo bounded history writer channel:

```text
SessionActor
   ↓ try_send / bounded send policy
HistoryWriter
   ↓
SQLite
```

Vì history là persistent side effect, cần định nghĩa failure policy.

Contract best-effort:

```text
history_tx.try_send(record)
  accepted       → async persist
  queue full     → drop record + metric
  channel closed → drop record + metric
  database error → drop record + metric
```

Không await queue capacity, retry loop trong `SessionActor`, fail turn, close WS hay
rollback voice flow. Partial exchange là chấp nhận được: user hoặc assistant có thể
persist riêng lẻ. `DialogueHistory` trong RAM là conversational correctness còn
SQLite `history_messages` là optional archival copy; HistoryWriter fail không thay đổi
RAM history, prompt composition, tool continuation hay turn outcome.

Retention cleanup dùng Unix milliseconds UTC thống nhất và cutoff tuyệt đối
`now_utc - retention_days`; chạy một lần khi startup rồi mỗi 24 giờ, ngoài
`SessionActor`, audio frame path hoặc writer terminal path:

```sql
DELETE FROM history_messages
WHERE created_at < ?;
```

Cleanup vẫn chạy khi database tồn tại và retention config hợp lệ, kể cả
`database.history.enabled = false`: tắt capture mới không biến dữ liệu cũ thành
retention vô hạn. Retention không dựa vào `session_id`, `turn_id` hay last access.

## 21.4. Assistant message

Chỉ enqueue assistant history write sau:

```rust
WriterEvent::TurnClosed {
    outcome: WriterTurnOutcome::Normal,
    ..
}
```

Không persist partial assistant nếu aborted/failed.

## 21.5. Message snapshot

Mỗi write phải mang snapshot:

```rust
pub struct HistoryWrite {
    pub session_id: String,
    pub device_id: i64,
    pub agent_id: i64,
    pub template_id: Option<i64>,
    pub sequence: i64,
    pub turn_id: String,
    pub role: HistoryRole,
    pub text: String,
    pub created_at: i64,
}
```

Nếu template switch đang pending nhưng chưa apply, message vẫn ghi `active_template_id` cũ.

---

# 22. History query APIs

Các endpoint trong section này là admin API, bắt buộc authentication kể cả homelab,
và mount khi `database.enabled=true && api.enabled=true`, độc lập
`database.history.enabled`. Capture OFF chỉ ngăn HistoryWriter ghi mới; archive cũ vẫn
read/purge được và retention cleanup vẫn chạy.

Các API nên có sẵn ngay từ data layer:

```text
GET /api/admin/history
GET /api/admin/history?session_id=<id>
GET /api/admin/history?device_id=<db-id>
GET /api/admin/history?agent_id=<id>
GET /api/admin/history?template_id=<id>
GET /api/admin/history?role=user
```

Danh sách session của Device không cần `device_sessions`:

```sql
SELECT
    session_id,
    MIN(created_at) AS started_at,
    MAX(created_at) AS last_message_at,
    COUNT(*) AS message_count
FROM history_messages
WHERE device_id = ?
GROUP BY session_id
ORDER BY started_at DESC;
```

Expose service method:

```rust
pub async fn list_sessions_for_device(
    &self,
    device_id: i64,
    page: PageQuery,
) -> Result<Page<HistorySessionSummary>, DatabaseError>;
```

Sau này API chỉ gọi method này, không viết lại SQL aggregation trong handler.

---

# 23. Transactions

Dùng transaction cho operation có nhiều table.

Ví dụ assign default Template:

```text
BEGIN
  clear old default for agent
  verify template assignment
  set new default
COMMIT
```

Provider binding update:

```text
BEGIN
  verify template
  verify provider
  verify provider.type
  upsert binding
  verify template has complete binding set
COMMIT
```

Agent delete/disable không nên để handler tự chạy nhiều queries rời rạc.

Repository/service sở hữu transaction boundary.

Transaction phải ngắn: external validation không cần consistency chạy trước `BEGIN`; trong
transaction chỉ giữ revision check, mutation, revision increment và audit success insert. Không
HTTP request, MCP call, provider loading, filesystem operation hay await external service giữa
`BEGIN` và `COMMIT`.

---

# 24. Delete policy

V1 chỉ soft-disable configuration resource qua PATCH `enabled=false` với `If-Match`:

```text
agents.enabled = 0
agent_templates.enabled = 0
providers.enabled = 0
mcp_servers.enabled = 0
devices.enabled = 0
```

V1 không mount DELETE cho Agent, Template, Provider, MCP Server hoặc Device. Disable
không xóa history và không revoke Voice Session đã admission.

History destructive operation duy nhất là `POST /api/admin/history/purge`, với exactly
một scope `device_id`, `session_id` hoặc string `all`. Scope `all` bắt buộc
`confirm: "PURGE_ALL_HISTORY"`; request thiếu/ambiguous scope không purge. Purge chỉ
DB operation, không đổi DialogueHistory hay profile của session đang mở. ADR-0055 ghi
contract này.

Retention expiry cũng là delete policy explicit: chỉ xóa transcript đã quá
`retention_days`, ngoài realtime path, và phải có test cho boundary thời gian.

---

# 25. API CRUD đề xuất

Không bắt buộc implement tất cả ngay phase database, nhưng repository/service phải support được.

## Agents

```text
GET    /api/admin/agents
POST   /api/admin/agents
GET    /api/admin/agents/:id
PATCH  /api/admin/agents/:id
GET    /api/admin/agents/:id/templates
PUT    /api/admin/agents/:id/default-template/:template_id
```

## Templates

```text
GET    /api/admin/templates
POST   /api/admin/templates
GET    /api/admin/templates/:id
PATCH  /api/admin/templates/:id
PUT    /api/admin/templates/:id/providers/:type
```

## Providers

```text
GET    /api/admin/providers
POST   /api/admin/providers
GET    /api/admin/providers/:id
PATCH  /api/admin/providers/:id
```

## MCP

```text
GET    /api/admin/mcp
POST   /api/admin/mcp
PATCH  /api/admin/mcp/:id
PUT    /api/admin/agents/:agent_id/mcp/:mcp_id
DELETE /api/admin/agents/:agent_id/mcp/:mcp_id
```

## Devices

```text
GET    /api/admin/devices
POST   /api/admin/devices
GET    /api/admin/devices/:id
PATCH  /api/admin/devices/:id
PUT    /api/admin/devices/:id/agent/:agent_id
```

## History

```text
GET    /api/admin/history
GET    /api/admin/devices/:id/history-sessions
GET    /api/admin/history/sessions/:session_id
POST   /api/admin/history/purge
```

---

# 26. API security boundary

DB query API có thể chứa prompt/history/config nên không expose unauthenticated chỉ vì server chạy homelab. Toàn bộ admin CRUD/query API ở DB-3 trở đi bắt buộc dùng
`Authorization: Bearer <admin_token>`; history API không có anonymous mode và hoàn
toàn tách `auth.token` của Voice/OTA.

```toml
[api]
enabled = false
admin_token = ""
```

Khi `api.enabled = true`, `admin_token.trim()` phải non-empty hoặc startup fail-fast;
credential thiếu/sai trả `401`. Chỉ chấp nhận đúng một `Authorization` header theo Bearer
scheme, token non-empty và constant-time compare; missing, malformed hoặc wrong đều `401`.
V1 không có app-level brute-force limiter; reverse proxy/network ACL chịu trách nhiệm throttling.
Khi API disabled, không mount `/api/admin` và client nhận `404`, không phải `401`. Không log,
debug hay persist `admin_token` xuống SQLite.

V1 `/api/admin/*` không emit CORS header, không hỗ trợ cookie auth và không wildcard
origin/credentials. Điều này không là network isolation: reverse proxy, network ACL và
same-origin UI vẫn là deployment responsibility. Web Manager/CORS allowlist là feature
config riêng sau này, không tự bật theo `api.enabled`.

Middleware `/api/admin` tạo `Uuid::new_v4()` server-side trước auth, dùng cùng ID cho
structured logs, `admin_audit_events.request_id`, error envelope và response header
`X-Request-Id`. Client-supplied `X-Request-Id`, `X-Correlation-Id` hay body request_id
không authoritative và có thể bị ignore. Auth failure có server request ID trong
telemetry/response nhưng không insert audit event.

Admin JSON mutation surface dùng một shared middleware/extractor seam, không lặp ở handler:

```rust
const MAX_ADMIN_JSON_BODY_BYTES: usize = 256 * 1024;
```

Order cố định là request ID → transport/body protection → admin auth → JSON extractor →
handler/service. Transport protection không parse/log content: chỉ cho `Content-Encoding` absent
hoặc `identity`, reject mọi encoding khác (`gzip`, `br`, `deflate`, ...) với
`415 unsupported_content_encoding`; Admin router không gắn decompression middleware V1. Với
endpoint mutation JSON, chỉ nhận `Content-Type: application/json` (cho phép valid media-type
parameter như `charset=utf-8`); missing/sai type trả `400 invalid_content_type`.

Body raw vượt 256 KiB trả `413 request_too_large`, không read/truncate prefix để parse. Sau khi
pass size/type, malformed JSON trả `400 invalid_json`; không trả raw serde error/body fragment.
Cap áp dụng Agent, Template, Provider, MCP Server, Device, binding mutation và History purge;
Q49 `config_json <=64 KiB` vẫn là resource-specific cap sâu hơn. GET surface không yêu cầu JSON
body; endpoint không hỗ trợ body có thể ignore hoặc reject theo API convention, nhưng không bypass
transport bound. Log chỉ event/reason/request ID, không body, parser fragment, config/prompt/
secret ref hay sensitive history selector.

`admin_audit_events` không có Admin API read endpoint trong V1. Audit chỉ phục vụ retention và
operator-side forensics; future query API cần pagination bound/redacted metadata contract riêng.

`secret_ref` là opaque `SecretRef` deployment reference, không phải secret manager: write chỉ
nhận `secret_ref: null` hoặc raw printable ASCII `1..=256` bytes; reject non-ASCII/control,
empty, whitespace-only và leading/trailing space mà không trim/normalize. Domain layer không
áp env-name hoặc resolver-specific regex; reject `api_key`, `secret_value` hay secret plaintext
field. PATCH field absent giữ reference,
`null` remove và string replace. GET chỉ trả `has_secret_ref: bool`, không trả
identifier. CRUD request không resolve/test secret và V1 không có Admin API endpoint để
resolve, test hoặc đọc secret. Response coarse chỉ có thể báo runtime unavailable, không tên
secret hay resolver backend.

PATCH dùng DTO typed với tri-state chung, không JSON Merge Patch hoặc `Option<Option<T>>` ad hoc:

```rust
enum Patch<T> {
    Absent,
    Set(T),
    Clear,
}
```

Serde map field missing thành `Absent`, value thành `Set`, `null` thành `Clear`; domain quyết định
Clear có hợp lệ không. `null` chỉ clear field nullable/clearable; immutable hoặc non-nullable clear
trả `400`. PATCH không normalize thành update ngầm hoặc last-write-wins.

External MCP GET trả auth redacted: `none`, hoặc `{ type: "bearer",
has_secret_ref: true }`, hoặc `{ type: "header", header_name: "x-api-key",
has_secret_ref: true }`; không trả secret_ref/resolved value.

## 26.1. Optimistic concurrency

Mọi mutable resource (`agents`, `agent_templates`, `providers`, `mcp_servers`,
`devices`) có `revision`, GET trả field này và PATCH/PUT bắt buộc
`If-Match: "<revision>"`. HTTP adapter parse header thành
`ExpectedRevision(u64)` cho service layer; body không mang raw SQL/version handling.

Mutation update row và increment revision trong cùng statement/transaction:

```sql
UPDATE agent_templates
SET prompt = ?, revision = revision + 1, updated_at = ?
WHERE id = ? AND revision = ?;
```

Không có affected row thì lookup existence: missing → `404`; tồn tại nhưng revision
khác → `409`; không silent overwrite. Response thành công trả revision mới. Điều này
cũng áp dụng khi provider mutation trả `requires_restart=true`.

Binding không cần revision riêng; transaction validate expected owner revision, mutate
binding, rồi increment owner và commit atomically:

```text
Agent ↔ Template assignment change  → agents.revision++
Template ↔ Provider binding change  → agent_templates.revision++
Agent ↔ MCP binding change          → agents.revision++
Device → Agent binding change       → devices.revision++
```

Success audit là transactional requirement: trong cùng transaction validate revision →
mutate resource/binding (hoặc purge) → increment revision nếu có → insert
`admin_audit_events(outcome=success)` → commit. Audit insert fail rollback mutation và
trả `503`. Revision conflict rollback trước, sau đó attempt audit `outcome=conflict`
best-effort trong transaction riêng; audit conflict fail chỉ metric, response vẫn `409`.
Database error trước success không tạo dependency audit retry vô hạn.

Không log:

- provider secret;
- MCP secret;
- full prompt nếu telemetry policy không cho;
- history message content trong tracing.

Tracing chỉ nên log:

```text
resource
id
count
elapsed_ms
error_kind
```

---

# 27. Validation service

Không dựa chỉ vào SQLite constraints.

Common resource bounds: Resource Key `<=64` bytes; Protocol Device Identity `<=128` bytes;
name `<=128` UTF-8 bytes; description `<=2048` UTF-8 bytes; language `<=32` bytes;
Agent/Template prompt `<=64 KiB` UTF-8; URL `<=2048` bytes. `metadata_json` và MCP static
headers JSON mỗi cái `<=16 KiB` UTF-8. Metadata JSON dùng shape validator `depth<=8`, aggregate
object-key + array-item nodes `<=256`; không là unbounded config bag. Static headers vẫn phải
pass protected-header/auth validation hiện có. Vượt bound trả coarse validation error, không echo
value/body.

Tạo application validation cho:

## Agent

- key tạo mới match `^[a-z][a-z0-9_]{0,63}$`, immutable sau CREATE; PATCH đổi key → `400 immutable_field`;
- name non-empty;
- name/description/prompt/language theo common resource bounds;
- default template thuộc Agent.

## Template

- key cùng format, immutable sau CREATE;
- prompt/language valid;
- name/description/prompt/language theo common resource bounds;
- template enabled mới được activate;
- đủ 4 provider bindings;
- provider types đúng.

## Provider

- key cùng format, immutable sau CREATE;
- adapter được compile vào binary;
- `config_json` recursive protected-key guard pass, deserialize được thành typed
  discriminator-specific adapter config với `deny_unknown_fields`, canonical serialize trước persist;
- raw UTF-8 `<=64 KiB`, JSON depth `<=16`, aggregate object-key + array-item nodes `<=512`;
- typed config credential-free, không arbitrary JSON/headers/options escape hatch;
- model/artifact validation theo provider hiện tại;
- `secret_ref` parse được thành opaque `SecretRef`; Provider availability chỉ resolve lúc startup và
  External MCP availability chỉ resolve lúc admission snapshot, không resolve trong CRUD
  request hay Admin API preflight/test V1.

## MCP

- key cùng format, immutable sau CREATE;
- URL parse hợp lệ;
- URL/description/static headers JSON theo common resource bounds;
- URL/scheme/network policy được phép qua `mcp.external.network`;
- timeout > 0;
- secret không nằm trong headers JSON.
- `none` chỉ cho secret_ref absent/null; `bearer` yêu cầu secret_ref; `header` yêu cầu
  secret_ref và header name ASCII hợp lệ/canonical lowercase.
- Header mode block `host`, `content-length`, `transfer-encoding`, `connection`,
  `upgrade`, `te`, `trailer`, `proxy-authorization`, `cookie`, `set-cookie`,
  `authorization`; `authorization` chỉ có bearer mode.

## Device

- `device_id` immutable opaque protocol identity: `1..=128` bytes, không NUL/ASCII control
  `0x00..=0x1F` hay `0x7F`, không trim/case-fold/Unicode or MAC normalize; Device đổi identity tạo row mới;
- Agent tồn tại và enabled.
- name/description/metadata JSON theo common resource bounds.

---

# 28. Typed provider config từ `config_json`

Không truyền JSON trực tiếp vào provider factory.

Flow:

```text
raw config_json (<=64 KiB)
  ↓
JSON parse + depth/node cap
  ↓
protected-key guard
  ↓
match adapter
  ↓
typed deserialize + deny_unknown_fields
hoặc GipformerConfig/OpenAiConfig/ZeroTtsConfig
  ↓
existing config validation
  ↓
provider factory
```

Ví dụ:

```rust
match row.adapter.as_str() {
    "silero_onnx" => {
        let cfg: SileroOnnxConfig = serde_json::from_str(&row.config_json)?;
        validate_silero(&cfg)?;
        // build through existing provider registry
    }
    "gipformer_sherpa_offline" => { /* ... */ }
    "openai" => { /* ... */ }
    "zerotts_onnx" => { /* ... */ }
    other => return Err(ProviderLoadError::UnknownAdapter(other.into())),
}
```

Cùng validator chạy trong Admin API write và startup loader. Generic shape violation trả
`provider_config_invalid` (client) và bounded telemetry reason; required row invalid fail
startup, optional/unbound invalid giữ unavailable và không block boot.

Mục tiêu là DB không phá typed provider contracts hiện tại.

---

# 29. Repository query pattern mẫu

Ví dụ list Agent reusable:

```rust
pub async fn list(&self, query: AgentListQuery) -> Result<Page<AgentRow>, DatabaseError> {
    let mut where_sql = Vec::new();
    let mut builder = sqlx::QueryBuilder::new(
        "SELECT id, key, name, description, enabled, created_at, updated_at FROM agents"
    );

    // Thực tế nên tạo helper condition builder nội bộ repository để bind values,
    // không format user input vào SQL string.

    // ... typed filters ...
    // ... validated sort enum ...
    // ... LIMIT/OFFSET ...

    todo!()
}
```

Không đưa helper thành generic raw SQL builder public cho API module.

---

# 30. Testing strategy

## 30.1. Migration tests

Test database mới:

```text
open temp SQLite
run migrations
PRAGMA foreign_keys=ON
verify all tables/indexes
```

Migration compatibility bắt buộc:

```text
empty DB                 → migrate latest → boot
older supported schema   → forward migrate → boot
latest schema            → no-op migration → boot
schema newer than binary → database_schema_incompatible trước listener
migration failure        → startup failure trước listener
restart migrated DB      → idempotent boot
```

Migrations được author transaction-safe khi SQLite operation cho phép. Failure không cho
listener bind và application không cố “chạy tiếp” partial schema; recovery/restore là thao tác
operator explicit.

## 30.2. Repository tests

Bắt buộc:

- create/get/update Agent;
- N:N Agent ↔ Template;
- one default Template per Agent;
- provider type binding validation;
- MCP N:N Agent binding;
- Device → Agent;
- history session grouping;
- same `session_id` sequence uniqueness.

## 30.3. Resolver tests

Cases:

```text
Agent without Template
  → server defaults

Agent with default Template
  → Template providers

Agent không có assignment
  → server defaults

Default assignment/template/provider/runtime invalid
  → controlled `503 Agent runtime unavailable`, không fallback

Non-default enabled assignment invalid
  → exclude TemplateSwitchCatalog + warning/metric, default session vẫn start

Template missing ASR binding
  → invalid, never create session with mixed defaults

Provider disabled
  → Template invalid
```

## 30.4. WebSocket tests

Bổ sung reference/integration tests:

1. known Device resolves assigned Agent;
2. default Template applies correct runtime IDs;
3. switch tool schedules Template B;
4. current turn stays Template A;
5. next turn uses Template B;
6. same WebSocket `session_id` used for all persisted history;
7. reconnect creates new `session_id`;
8. aborted assistant response không persist;
9. normal writer close persist assistant text.
10. admin mutation không đổi `EffectiveSessionProfile` của session đã admit.
11. session mới thấy Database Desired Configuration đã mutate.
12. non-default invalid Template bị exclude nhưng default valid vẫn admit.
13. `switch_template` không query SQLite và không nhìn thấy admin mutation sau admission.
14. SessionProfileRevision chỉ tăng sau successful normal-boundary switch.
15. mixed Device/External multi-tool round giữ model order và đúng một terminal result/call.
16. External MCP success text/JSON pipeline reject unsupported, mixed hoặc oversize result.
17. over-cap round executes zero call; round-count cap rejects next round before execution.
18. tool budget clamps call timeout, cancellation discards late response và không continuation.

## 30.5. API infrastructure tests

Test common components một lần:

- page/page_size clamps;
- invalid sort rejected;
- DB not-found → 404;
- unique conflict → 409;
- query values always bound;
- response envelope consistent.
- `api.enabled=false` không mount admin router (`404`);
- `api.enabled=true` với admin token trim-rỗng fail startup;
- admin credential thiếu/sai → `401`.
- exactly one strict Bearer Authorization header uses constant-time comparison; malformed/missing
  header still `401`; Admin V1 has no in-process brute-force limiter.
- GET revision, `If-Match` success trả revision mới, stale `If-Match` → `409`.
- binding mutation increment đúng owner revision atomically.
- admin server-generated request_id có trong response/error/audit; supplied client ID không authoritative.
- admin V1 không emit CORS/cookie auth; audit cleanup failure không đổi mutation outcome.
- Admin JSON body `<=256 KiB`; oversized → `413 request_too_large`, wrong/missing JSON content
  type → `400 invalid_content_type`, malformed JSON → `400 invalid_json`, non-identity content
  encoding → `415 unsupported_content_encoding`; no decompression middleware bypass.
- V1 không mount hard DELETE configuration resources; history purge require exact scope
  và `PURGE_ALL_HISTORY` confirmation khi scope all.
- GET không lộ `secret_ref`; write plaintext secret field bị reject.
- Provider config recursive protected-key guard reject mọi depth/array; exact key matching vẫn
  accept `max_tokens`, `tokenizer`, `token_budget`; startup validate lại persisted row.
- protected-key regression vectors: `api_key`, `apiKey`, `API_KEY`, `clientSecret`,
  `client_secret`, nested `password`, array `token`, `Authorization`, `secret_ref` reject;
  malformed/unknown adapter field bị `deny_unknown_fields` reject.
- config shape boundary: 64 KiB/depth 16/node 512 pass và từng cap + invalid JSON reject với
  client error `provider_config_invalid`; Admin/startup cùng pure validator result.
- SQLite busy/locked sau `busy_timeout` map Admin `503 database_busy`, pool acquire timeout và
  storage failure map `503 database_unavailable`; không application retry/transaction retry.
- history/audit retention contention abort current run/drop record, không requeue; scheduled run
  sau mới thử lại.
- fresh DB creates schema/index only; no implicit Agent/Template/Device seed; admission enabled
  before provisioning returns `403`.
- Patch tri-state covers absent/set/clear; null immutable/non-nullable field → `400`.
- field/query bounds: common resource fields, metadata/header JSON, page size/filter/search/sort
  limits reject before persist/query; no raw sort/filter expression.
- shutdown stops new admissions/tool calls, drains within configured grace then controlled-closes;
  readiness checks owned dependencies only and optional MCP failure remains ready.
- External MCP HTTPS validates normal certificate/hostname, no insecure TLS mode; audit has no
  V1 Admin read route.
- immutable Resource Key/Protocol Device Identity validation và `400 immutable_field`.
- history read/purge vẫn mount khi capture OFF; retention vẫn cleanup archive cũ.
- admin audit mutation/conflict metadata không chứa sensitive field; auth failure chỉ telemetry.
- success mutation/purge audit insert rollback-safe; conflict audit failure giữ response `409`.

Sau đó resource endpoint tests chỉ tập trung domain semantics.

---

# 31. Observability

Add spans/events:

```text
db_query
  repository
  operation
  elapsed_ms
  row_count

db_write_failed
  entity
  operation
  error_kind

session_profile_resolved
  device_db_id
  agent_id
  template_id
  provider_vad_id
  provider_asr_id
  provider_llm_id
  provider_tts_id
```

Không log history text hoặc prompt content mặc định.

History metrics không có `session_id`, `turn_id` hay text label:

```text
history_write_enqueued_total{role}
history_write_dropped_total{role,reason}
history_write_failed_total{role,reason}
history_queue_depth
```

`reason` là bounded enum: `queue_full`, `writer_closed`, `database_error`, `shutdown`.
Log drop chỉ có `event=history_write_dropped`, `role`, `reason`.

External MCP metrics:

```text
mcp_resolve_success_total{server_key,outcome="success"}
mcp_resolve_failure_total{server_key,outcome="failure",reason}
mcp_resolve_duration_ms{server_key,outcome}
external_mcp_session_tool_cap_exceeded_total
external_mcp_tool_calls_total{server_key,outcome}
external_mcp_tool_call_duration_ms{server_key,outcome}
```

Tool-call outcome là bounded `success`, `timeout`, `unavailable`, `invalid_response`,
`protocol_error`; labels không chứa arguments, session_id hay turn_id. Failure log chỉ
event, server_key, optional tool name theo privacy policy và reason; không body/secret/URL.

Telemetry seam là process-owned và bounded: caller chọn metric từ tập cố định và label từ bounded
class. Giá trị tự do duy nhất là `server_key`, vốn đã bị Admin API bound ở 64 byte `[a-z0-9_]`;
destination, header value, credential, protocol session id, tool argument và tool result không
phải tham số của seam nên không có cách nào trở thành label. `mcp_resolve_duration_ms` được gắn
cùng `server_key` và `outcome` với counter nó đi cùng, vì một duration không có label thì không
quy được về server nào. Aggregate cap được đếm riêng là
`external_mcp_session_tool_cap_exceeded_total`: nó là một sự kiện của cả snapshot chứ không phải
của một server, nên không được đếm như server resolve failure và không emit per-server duration.
Mỗi server đã discovery thành công vẫn được đếm là resolved, dù snapshot aggregate sau đó bị
reject. ADR-0056 là nguồn chốt.

---

# 32. Performance rules

SQLite không được nằm trên audio frame path.

Sai:

```text
Opus frame
  → SELECT active template
  → VAD
```

Đúng:

```text
WS connect / safe template switch boundary
  → DB resolve
  → immutable session snapshot

Opus frames
  → memory/runtime only
```

History write dùng bounded async writer để DB latency không chặn VAD/ASR/TTS path.

API queries dùng pool riêng cùng `SqlitePool`; WAL giúp reader chạy đồng thời với writer tốt hơn.

---

# 33. Suggested module dependency direction

```text
app/api
   ↓
service
   ↓
repository
   ↓
database/sqlx

websocket bootstrap
   ↓
session config service
   ↓
repository

SessionActor
   ↓
typed history command / template-switch command
   ↓
service boundary
```

Không cho:

```text
providers → API
protocol → SQLx
session core → Axum
repository → SessionActor
```

---

# 34. Implementation phases

## Phase DB-1 — Foundation

- add SQLx;
- `DatabaseConfig`;
- pool/WAL/migrations;
- database error;
- AppState database handle;
- migration tests.

Không thay đổi WebSocket admission, provider resolution, Agent, Template, MCP hay
History runtime.

Completion gate:

```text
server boot
  → DB open
  → migrations complete
  → /health ok
```

Release/migration procedure:

```text
1. stop hoặc quiesce writes khi cần
2. tạo SQLite-consistent backup (SQLite backup API/CLI), hoặc stop process trước khi copy
3. deploy binary mới
4. startup apply forward migrations
5. verify /health + readiness
```

WAL database không được backup bằng cách chỉ copy `.db` khi process đang ghi. Application V1
không auto-backup, auto-restore, rotate backup hoặc downgrade schema. Rollback release là stop
server → restore backup compatible → deploy old binary.

## Phase DB-2 — Typed repositories + query infrastructure

Implement 10 tables:

```text
agents
agent_templates
agent_template_assignments
providers
template_provider_bindings
mcp_servers
agent_mcp_bindings
devices
history_messages
admin_audit_events
```

Implement typed repositories, pagination/filter/sort/error/response primitives và
repository tests. Không đăng ký admin CRUD API hay thay đổi Voice behavior.

## Phase DB-3 — Admin CRUD/query API

Implement:

```text
PageQuery
Page<T>
ApiResponse<T>
ApiError
SortDirection
typed filters
repository pagination conventions
/api/admin router với authentication bắt buộc
```

Expose Agent / Template / Provider / MCP / Device CRUD/query. Provider mutation
runtime-affecting persist desired configuration và trả `requires_restart: true`;
không hot-reload `RuntimeCatalog`. Completion gate: một resource mới có thể thêm
list/get endpoint mà không tạo lại DB/error/pagination infrastructure.

## Phase DB-4 — Device → Agent resolution

Implement database-backed device admission:

```text
Device → Agent
```

Unknown/disabled Device reject `403` trước WebSocket upgrade khi `database.enabled = true`
và `database.devices.admission_enabled = true`; invalid Agent profile hoặc DB/pool/resolver
unavailable trả `503`, không fallback. `auto_register = true` chỉ dev/migration
và bắt buộc `auto_register_agent_key` resolve tới đúng một Agent enabled; Device mới
`enabled=true`, có Agent binding, metadata audit safe và UNIQUE/upsert-safe race handling.

## Phase DB-5 — Template → Provider runtime resolution

Implement:

```text
Device → Agent → Template → Providers
```

Fallback server config chỉ khi Agent không có assignment. Khi assignment tồn tại,
phải có exactly one enabled default và Template hoàn chỉnh với các binding resolve tới
validated, already-loaded `RuntimeCatalog`; mọi invalid state trả `503 Agent runtime
unavailable`, không hot load và không query DB trên audio path.

## Phase DB-6 — Template switch tool

- internal LLM-visible tool;
- Agent assignment validation;
- pending switch;
- next-turn apply;
- tests across tool continuation boundary;
- reject a template whose runtime is unavailable.

## Phase DB-7 — External MCP Streamable HTTP

- DB config;
- application-owned MCP manager;
- Agent bindings;
- fail-soft resolve/tool merge policy, diagnostics và metrics;
- typed `none`/`bearer`/`header` auth injection với secret_ref redaction;
- protected static header reject sau canonicalization và no post-auth header insertion;
- supported-schema allowlist/cap vectors reject unsupported/recursive/over-budget tool;
- one logical External ToolCall emits at most one outbound attempt and terminal synthetic failure result;
- schema `required` default false nhưng V1 chỉ support false;
- `self.*` Device MCP và `external.<server_key>.*` External MCP namespace;
- fresh parallel-bounded admission resolve, không reuse old tools/list snapshot;
- raw catalog caps per-server/per-session/page/schema/description, aggregate overflow
  exclude toàn bộ External MCP snapshot;
- outbound hostname/CIDR DNS-revalidated allowlist, no redirect/query/userinfo URL policy;
- do not change Device MCP semantics.

## Phase DB-8 — Optional text history

- default `database.history.enabled = false`;
- bounded HistoryWriter only when enabled;
- final user text and assistant only after normal writer close;
- same `session_id` per connection;
- retention/delete outside realtime path;
- authenticated admin history API;
- `try_send` drop/metric policy và RAM-history independence;
- ADR-0048 privacy update.

---

# 35. Definition of Done

Database feature chỉ hoàn tất khi thỏa tất cả:

1. SQLite WAL + FK + migrations chạy khi startup.
2. Không query SQLite trong PCM/audio frame path.
3. Agent ↔ Template là N:N.
4. Mỗi Agent tối đa một default Template enabled.
5. Template có prompt + language + đủ VAD/ASR/LLM/TTS binding.
6. Agent không có Template assignment dùng server defaults hoàn toàn; Agent đã có
   assignment nhưng default/template/provider/runtime invalid fail closed `503`, không fallback.
7. Provider DB config được deserialize về typed adapter config trước khi build.
8. Provider runtime được application-owned và reuse qua `RuntimeCatalog`.
9. Device bind một Agent; DB-4 admission chỉ active khi `database.enabled = true` và
   `database.devices.admission_enabled = true`; unknown/disabled Device trả `403`,
   profile/DB resolver unavailable trả `503` trước upgrade, không fallback.
10. Active Template thuộc WebSocket session, không thuộc Agent global.
11. Template switch qua tool chỉ apply từ turn kế tiếp.
12. MCP Streamable HTTP config thuộc Agent và tách khỏi Device MCP.
13. External MCP unavailable fail-soft trong V1, không đóng Voice Session hay advertise
    stale tools; `agent_mcp_bindings.required` chỉ là schema extension default false.
14. Không có `device_sessions` table.
15. History default OFF; khi enabled, `history_messages.session_id` group toàn bộ
    message cùng WS connection và retention cleanup nằm ngoài realtime path.
16. Khi history enabled, user history chỉ là final text.
17. Khi history enabled, assistant history chỉ persist sau `WriterTurnOutcome::Normal`.
18. HistoryWriter là best-effort `try_send`; drop/failure không đổi DialogueHistory,
    prompt, tool continuation hay turn outcome.
19. API handlers không viết SQL trực tiếp.
20. Connection pool, pagination, filtering conventions, sorting, API error và response envelope được reuse.
21. Không expose arbitrary/raw SQL API.
22. Thêm API query mới không cần viết lại DB initialization/error/pagination infrastructure.
23. Migration/repository/resolver/WebSocket/API tests đều pass; history tests bao gồm
    disabled path, retention và authenticated admin access.
24. Admin API chỉ mount khi `api.enabled = true`, bắt buộc admin token riêng và startup
    fail-fast khi token rỗng; API disabled trả `404`.
25. `EffectiveSessionProfile` immutable đến disconnect; admin mutation chỉ ảnh hưởng
    session mới, realtime revoke là feature command/event riêng.
26. TemplateSwitchCatalog chỉ chứa candidate enabled đã validate/load ở admission;
    default invalid fail `503`, non-default invalid bị exclude có diagnostics.
27. Tool registry giữ ToolOrigin typed, External MCP dùng namespace `external.<server_key>.*`
    và không có bind-order collision resolution.
28. Agent không assignment có TemplateSwitchCatalog rỗng và không advertise switch tool;
    sanitizer External MCP là pure, bounded và có deterministic test vectors.
29. External MCP snapshot fresh mỗi admission, có per-server/overall bounded budget;
    admin mutation dùng `If-Match` optimistic concurrency và owner revision transaction.
30. Configuration resources V1 soft-disable only; history purge có explicit scope/confirm
    và secret references opaque write/boolean read.
31. DB-backed admission fail closed khi enabled: unknown/disabled Device `403`, invalid
    profile/infrastructure unavailable `503`; raw External MCP catalog caps không truncate.
32. History archive read/purge lifecycle độc lập capture; Resource Key/Protocol Device
    Identity immutable và External MCP outbound pass network policy sau DNS resolution.
33. `database.enabled=true` fail-fast trước bind, còn runtime DB failure không đổi liveness
    hay Effective Session Profile đã admit.
34. Startup-required provider set load trước listener; unbound provider không block boot;
    admin audit retention độc lập và không ghi sensitive request/config data.
35. Provider Load Plan required fail-fast, optional attempt-load fail-soft/exclude candidate;
    success admin mutation/purge cùng transaction audit, static MCP headers không bypass typed auth.
36. Runtime response tách status/process match desired revision; Admin API dùng server request ID
    và External MCP schema chỉ nhận bounded supported subset.
37. Admin API no-CORS/cookie V1, audit maintenance non-critical; External ToolCall không retry
    và terminal synthetic result giữ LLM continuation/Voice Session sống.
38. Shared Tool-round executor chạy strict sequential cho `self.*`/`external.*`; External MCP
    success result chỉ Text/Structured JSON bounded, không partial accept/persist/log body.
39. Tool-round cap validate upfront; execution budget/cancellation không start side-effect mới,
    không retry và không append late ToolResult/continuation.
40. `llm.tools` chỉ nhận `1..=32` calls/round, `1..=8` rounds/turn và `1..=120_000` ms
    turn execution budget; invalid config fail startup trước listener.
41. Session Tool Catalog immutable đến disconnect; External MCP call failure không remove
    capability, không mutate DB/runtime availability và V1 không có circuit breaker.
42. V1 Agent→External MCP binding publish mọi tool đã validate của server; External MCP
    calls có semaphore global `1..=64` permits/server (default `16`), wait tính vào turn
    budget và quá tải không gửi outbound request.
43. `SecretResolver` deployment-owned là sole runtime resolver; V1 `EnvSecretResolver` mới
    hiểu opaque reference là env name. Required Provider secret failure fail startup, optional
    Provider/MCP failure fail-soft; Admin API không resolve/test/đọc secret.
44. Provider secret snapshot theo RuntimeCatalog và chỉ thấy rotation sau restart; External
    MCP credential thuộc `ExternalMcpClient` trong WS snapshot và chỉ session mới thấy rotation.
    `401`/`403` tools/call là `external_tool_auth_failed`, không refresh/retry/mutate catalog.
45. `SecretRef` là opaque printable ASCII `1..=256` bytes, không trim/normalize hay áp
    resolver syntax; Env resolver tự validate env-name và redacts mọi reference ở observability.
46. `providers.config_json` là canonical typed non-secret config, protected-key guard recursive
    + `deny_unknown_fields` không có arbitrary escape hatch; startup validate lại persisted row.
47. Provider config raw UTF-8 `<=64 KiB`, depth `<=16`, aggregate key/item nodes `<=512`;
    một `ProviderConfigValidator` chạy trước typed deserialize cho Admin và startup.
48. SQLx migration history authoritative và forward-only; DB schema newer than binary hoặc
    migration failure fail startup trước listener, rollback/backup/restore là operator-owned.
49. SQLite contention chỉ chờ `busy_timeout` `1..=30_000` ms; pool exhaustion taxonomy riêng,
    không application retry và không SessionActor block SQLite.
50. Admin JSON mutation body `<=256 KiB` trước auth/deserialization; V1 reject compressed body
    và dùng shared content-type/size/JSON extractor errors, không log content.
51. SQLite V1 is single-process/local-filesystem owner with explicit bootstrap; PATCH typed
    tri-state, resource/query caps, bounded shutdown, dependency-only readiness, strict Admin
    Bearer, verified MCP TLS và audit no-read route giữ deploy/API lifecycle deterministic.

---

# 36. Target architecture cuối cùng

```text
                         ┌──────────────────────────┐
                         │       config.toml        │
                         │ server/provider defaults │
                         └────────────┬─────────────┘
                                      │
                                      │ fallback
                                      │
┌────────────┐       ┌─────────────┐  │
│  Device    │──────▶│    Agent    │──┘
└────────────┘       └──────┬──────┘
                            │
                  ┌─────────┴──────────┐
                  │                    │
                  ▼                    ▼
           ┌──────────────┐      ┌──────────────┐
           │   Template   │      │ MCP Servers  │
           └──────┬───────┘      │ Stream HTTP  │
                  │              └──────────────┘
                  ▼
        ┌─────────────────────┐
        │ Provider Bindings   │
        │ VAD ASR LLM TTS     │
        └──────────┬──────────┘
                   │
                   ▼
        ┌─────────────────────┐
        │ RuntimeCatalog      │
        │ application-owned   │
        └──────────┬──────────┘
                   │
                   ▼
        ┌─────────────────────┐
        │    SessionActor     │
        │ active template     │
        │ pending template    │
        │ session_id          │
        └──────────┬──────────┘
                   │
           final text only
                   │
                   ▼
        ┌─────────────────────┐
        │ history_messages    │
        │ group by session_id │
        └─────────────────────┘

HTTP Admin/API
     │
     ▼
Typed API Layer
     │
     ▼
Service Layer
     │
     ▼
Repository Layer
     │
     ▼
SQLite / SQLx
```

Kiến trúc này giữ realtime voice path độc lập với DB, đồng thời chuẩn bị sẵn một data/API layer để sau này thêm manager web, CRUD hoặc query endpoint mà không phải thiết kế lại connection, pagination, filtering, error handling và database access từ đầu.
