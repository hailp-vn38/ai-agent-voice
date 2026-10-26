# Multi-Channel Notification Hub (Web UI, Telegram Bot, Zalo Bot) — Hướng dẫn Triển khai Chi tiết

> **Repository:** `voice-agent-server`  
> **Đối chiếu code:** nhánh hiện tại (`ADR 0127` + commit `39988c2a` — unified `ReminderCoordinator`, `orchestrator-flow`, `llm-wiki-rs`, `voice-agent.db`)  
> **Kỷ luật thiết kế:** `/codebase-design` (**Module**, **Interface**, **Implementation**, **Depth**, **Seam**, **Adapter**, **Leverage**, **Locality**) & `/domain-modeling`  
> **Mục tiêu:** Xây dựng một **Deep Module** duy nhất (`NotificationHub`) quản lý thông báo đa kênh: mặc định lưu bền vững & phát thời gian thực lên **Admin Web UI** (`web_ui`), đồng thời định tuyến bất đồng bộ qua **Telegram Bot** (`telegram`) và **Zalo Bot** (`zalo`) theo thiết lập của Admin.

---

## 1. Phạm vi, Từ vựng Domain & Quyết định Thiết kế

### 1.1. Từ vựng Domain chuẩn hóa (`## Multi-Channel Notifications`)

Để tách biệt hoàn toàn với [`DeviceNotifications`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/database/notifications/mod.rs#L111-L115) và [`ReminderCoordinator`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/services/notifications/mod.rs#L21-L25) (vốn đang cài đặt **Reminder** thuộc sở hữu của một **Device** để phát **Direct Speech** qua phiên WebSocket), tài liệu này định nghĩa bộ thuật ngữ domain thuần túy (không chứa chi tiết cài đặt kỹ thuật) cho mục `## Multi-Channel Notifications` trong [`GLOSSARY.md`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/GLOSSARY.md) và [`CONTEXT.md`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/CONTEXT.md):

- **Notification**: Thông điệp bền vững phản ánh một sự kiện cần chú ý của hệ thống (Background Task kết thúc, lịch hẹn đến hạn, hoặc yêu cầu duyệt quản trị), độc lập với Reminder phát giọng nói trên Device.
  - _Avoid_: `Device Reminder`, `Job Change Signal`, `Tool Execution Status`
- **Notification Inbox**: Hộp thư thông báo mặc định dành cho Admin trên giao diện quản trị, duy trì trạng thái chưa đọc/đã đọc của từng Notification.
  - _Avoid_: `ephemeral toast`, `Job Event log`
- **Notification Channel**: Kênh đích nhận Notification, gồm kênh mặc định trên giao diện quản trị và các kênh nhắn tin bên ngoài đã cấu hình.
  - _Avoid_: `Direct Speech`, `Voice Session`
- **Notification Channel Policy**: Quyết định quản trị xác định kênh thông báo nào được bật, đích nhận và những chủ đề sự kiện nào được phép chuyển phát qua kênh đó.
  - _Avoid_: `Agent Tool Allowlist`, `Reminder policy`
- **Channel Delivery Attempt**: Một lần thử chuyển phát một Notification tới một Notification Channel bên ngoài, có kết quả độc lập với trạng thái đã đọc trong Notification Inbox.
  - _Avoid_: `Delivery Attempt` (của Device Reminder)

> **Quan hệ với `DeviceNotifications` (`device_reminders`), `ReminderCoordinator` & `TaskManager` (`tasks.db`)**:
> - Từ commit `39988c2a`, toàn bộ công cụ giọng nói đặt lịch nhắc (`create_reminder`, `list_reminders`, `update_reminder`, `cancel_reminder` tại [`tools/builtin.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/tools/builtin.rs#L31-L82)) cùng REST API thiết bị đều hội tụ về [`ReminderCoordinator`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/services/notifications/mod.rs#L21-L25) và bộ lập lịch hướng sự kiện [`services::reminder_scheduler::start`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/services/notifications/reminder_scheduler.rs#L18-L90) (hàm `deliver_task_reminder_to_devices` cũ trong `app/mod.rs` đã được gỡ bỏ).
> - Bảng `device_reminders` và `services::reminder_scheduler::start` tiếp tục phục vụ hợp đồng giao thức phần cứng (**Device Cache ACK**, **Playback ACK**, **Direct Speech** theo `ADR-0087..0091`) và giữ nguyên không thay đổi.
> - Để đảm bảo Admin luôn nhận được cảnh báo trên **Web UI**, **Telegram Bot** và **Zalo Bot** (ngay cả khi thiết bị vật lý hoặc Virtual Device đang offline), chủ đề `NotificationTopic::ReminderDue` được cấp dữ liệu từ **2 nguồn bổ trợ nhau**:
>   1. **Từ `services::reminder_scheduler::start`** ([`reminder_scheduler.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/services/notifications/reminder_scheduler.rs#L48-L85)): Khi một `DueReminder` được claim đến hạn (`claim_next_due_for_device`) hoặc hết hạn chờ thiết bị kết nối, scheduler đồng thời gọi `notification_hub.publish(...)` (chống trùng lặp bằng `dedup_key = "device_reminder_due:{reminder_id}"`).
>   2. **Từ `TaskManager` (`tasks.db`)** trong [`prepare_background`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/app/mod.rs#L451-L473): Khi một lịch nhắc/schedule thuộc `orchestrator-flow` phát `TaskEvent::ReminderDue { schedule }`, `prepare_background` chuyển tiếp sự kiện sang `notification_hub.publish(...)`.

### 1.2. Phạm vi thực hiện

**Có làm:**
- Lưu bền vững mọi **Notification** vào `voice-agent.db` (migration `0137_notification_hub.sql`) với khóa chống trùng lặp (`dedup_key`) để đảm bảo tính idempotent.
- Kênh mặc định **Web UI** (`web_ui`): Luôn bật; hỗ trợ xem danh sách phân trang, lọc `unread`, đánh dấu đã đọc (`mark_read` từng mục hoặc tất cả), xóa thông báo (`delete_inbox`), và nhận sự kiện tức thời qua SSE (`GET /api/admin/notifications/stream`) để hiển thị **Badge đếm chưa đọc** + **Toast Notification**.
- Kênh ngoại vi **Telegram Bot** (`telegram`) và **Zalo Bot** (`zalo`):
  - Lưu trữ `bot_token` an toàn bằng [`CredentialCipher`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/database/security/credentials.rs#L64-L67) / [`SecretResolver`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/database/security/secrets.rs#L98-L103) với kiểu cập nhật tường minh [`Patch<SecretValue>`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/app/admin/mod.rs#L628-L642) (write-only khi `PUT`, redacted metadata khi `GET`, hỗ trợ fallback biến môi trường `VOICE_NOTIFICATION_TELEGRAM_BOT_TOKEN` và `VOICE_NOTIFICATION_ZALO_BOT_TOKEN`).
  - Cố định endpoint upstream chính thức của Telegram (`https://api.telegram.org`) và Zalo Bot (`https://bot-api.zapps.me`) trong production code; **không** cho phép ghi đè `api_base_url` qua Admin API nhằm ngăn chặn tấn công đánh cắp `Resource Credential` sang máy chủ giả mạo.
  - Ghi nhận **Admin Audit Event** (qua [`database/mutations/audit.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/database/mod.rs#L24)) cho mọi thao tác cập nhật cấu hình kênh hoặc khi xảy ra xung đột `409 Conflict` (`If-Match`).
  - Hàng đợi bền vững (`notification_outbox`) với trạng thái chuẩn (`pending`, `sending`, `sent`, `failed`) kèm background worker gửi bất đồng bộ, có timeout 10s mỗi lần gọi, tối đa 4 lần thử với exponential backoff (`5s`, `30s`, `120s`), hoàn toàn không làm chậm luồng `TaskManager` hay kênh **Web UI**.
  - Nút **"Gửi thông báo thử nghiệm"** (`POST /api/admin/notifications/channels/{channel}/test`) hỗ trợ kiểm tra cả cấu hình đang nhập trên form (draft) lẫn cấu hình đã lưu.

**Không làm:**
- Không thay đổi giao thức WebSocket của **Device** hoặc cơ chế claim/ACK **Direct Speech** trong [`ReminderCoordinator`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/services/notifications/mod.rs#L21-L25) và [`services::reminder_scheduler::start`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/services/notifications/reminder_scheduler.rs#L18-L90).
- Không làm bot hội thoại 2 chiều (webhook nhận tin nhắn chat từ Telegram/Zalo để điều khiển ngược lại server) trong phase này; chỉ thực hiện chiều gửi thông báo (outbound notification delivery).

---

## 2. Thiết kế Deep Module theo `/codebase-design`

### 2.1. Tại sao gom vào một Deep Module duy nhất (`NotificationHub`)?

Nếu để [`app/mod.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/app/mod.rs), [`services/notifications/reminder_scheduler.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/services/notifications/reminder_scheduler.rs), [`app/admin/background.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/app/admin/background.rs), [`app/admin/knowledge_processing.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/app/admin/knowledge_processing.rs) hoặc [`tools/system_mcp/`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/tools/system_mcp/mod.rs) tự đọc cấu hình kênh, tự format tin nhắn cho Telegram/Zalo và tự gọi HTTP, codebase sẽ sinh ra nhiều **shallow modules** với logic phân tán.

Thiết kế **`NotificationHub`** tại `crates/voice-agent-server/src/services/notification_hub/` đạt **Depth** cao:
- **Small Interface**: Toàn bộ các nơi phát sinh sự kiện chỉ gọi **1 hàm duy nhất** `hub.publish(NotificationInput)`. Các route Admin chỉ gọi 5 hàm quản trị (`list_inbox`, `mark_read`, `delete_inbox`, `get_settings` / `update_settings`, `test_channel`).
- **Deep Implementation**: Ẩn hoàn toàn bên trong module các chi tiết phức tạp:
  1. Kiểm tra `dedup_key` chống ghi trùng.
  2. Ghi nguyên tử (atomic transaction) vào cả `notifications_inbox` (cho Web UI) và `notification_outbox` (cho các kênh ngoại vi đang `enabled` và khớp `subscribed_topics`).
  3. Phát sự kiện real-time qua `tokio::sync::broadcast` tới các trình duyệt đang mở SSE.
  4. Đánh thức (`tokio::sync::Notify`) worker chạy nền để chuyển phát các bản ghi `notification_outbox` tới **Adapter** của Telegram và Zalo kèm giải mã `SecretRef` ngay trước thời điểm gửi.
- **Leverage & Locality**: Mọi thay đổi về cách format tin nhắn Telegram/Zalo, chính sách retry, hay bổ sung kênh mới chỉ diễn ra tại một nơi duy nhất (**Locality**), trong khi mọi caller hưởng lợi ngay lập tức (**Leverage**).

### 2.2. Phân loại Dependency & Đặt Seam (`DEEPENING.md`)

```text
┌───────────────────────────────────────────────────────────────────────────┐
│                   NotificationHub (Deep Module)                           │
│                                                                           │
│  External Seam (Public Interface):                                        │
│    • publish(NotificationInput)                                           │
│    • subscribe_web() -> broadcast::Receiver<NotificationSseEvent>         │
│    • list_inbox / mark_read / delete_inbox                                │
│    • get_settings / update_settings / test_channel                        │
├───────────────────────────────────────────────────────────────────────────┤
│  Implementation:                                                          │
│    • Event Producers Bridge (TaskEvent, Wiki Review, System Change Req)   │
│    • SQLite Store (Inbox + Outbox + Channel Settings + CAS + Audit)       │
│    • Secret Sealing & Resolution (via injected Arc<dyn SecretResolver>)   │
│    • Outbox Worker Loop (Claim -> Format -> Send -> Retry/Complete)       │
├───────────────────────────────────────────────────────────────────────────┤
│  Internal Seam (Injected Port): trait OutboundChannelAdapter              │
└───────────────┬───────────────────────────┬───────────────────────────────┘
                │                           │
     ┌──────────▼──────────┐     ┌──────────▼──────────┐     ┌─────────────────────────┐
     │ TelegramBotAdapter  │     │   ZaloBotAdapter    │     │ RecordingChannelAdapter │
     │  (Production HTTP)  │     │  (Production HTTP)  │     │   (Test / Qualification)│
     └─────────────────────┘     └─────────────────────┘     └─────────────────────────┘
```

1. **SQLite Inbox/Outbox & Web UI SSE (`Category 2: Local-substitutable`)**:
   - Sử dụng trực tiếp [`Database`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/database/mod.rs#L48-L51) (`voice-agent.db`) và `tokio::sync::broadcast`.
   - Không tạo trait repository giả; các bài test tích hợp chạy trực tiếp trên SQLite tạm thời qua **External Seam** (tức **Interface** của `NotificationHub`).
2. **Telegram Bot API & Zalo Bot API (`Category 4: True External`)**:
   - Là dịch vụ mạng bên thứ ba không thuộc quyền kiểm soát của tiến trình.
   - Đặt một **Internal Seam** (injected port) tại `trait OutboundChannelAdapter: Send + Sync`.
   - Có **3 Adapters** thỏa mãn Seam này (đảm bảo nguyên tắc *"Two+ adapters means a real seam"*):
     - `TelegramBotAdapter`: Gửi HTTP POST tới Telegram Bot API (`https://api.telegram.org/bot{token}/sendMessage`).
     - `ZaloBotAdapter`: Gửi HTTP POST tới Zalo Bot Platform API (`https://bot-api.zapps.me/bot{token}/sendMessage`).
     - `RecordingChannelAdapter`: Adapter thu giữ tin nhắn trong bộ nhớ dành cho test tự động (`scripts/agent-test.sh`) mà không phát sinh kết nối mạng ra ngoài.

---

## 3. Thiết kế Cơ sở dữ liệu (`0137_notification_hub.sql`)

Tạo file migration mới tại [`crates/voice-agent-server/migrations/0137_notification_hub.sql`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/migrations/0137_notification_hub.sql):

```sql
-- 1. Cấu hình kênh thông báo (Singleton state + Per-channel policy)
CREATE TABLE notification_settings_state (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    revision INTEGER NOT NULL DEFAULT 1 CHECK (revision > 0),
    web_ui_toast_enabled INTEGER NOT NULL DEFAULT 1 CHECK (web_ui_toast_enabled IN (0, 1)),
    retention_days INTEGER NOT NULL DEFAULT 30 CHECK (retention_days BETWEEN 1 AND 365),
    updated_at_ms INTEGER NOT NULL
);

INSERT INTO notification_settings_state (id, revision, web_ui_toast_enabled, retention_days, updated_at_ms)
VALUES (1, 1, 1, 30, 0);

CREATE TABLE notification_channel_configs (
    channel_kind TEXT PRIMARY KEY CHECK (channel_kind IN ('telegram', 'zalo')),
    enabled INTEGER NOT NULL DEFAULT 0 CHECK (enabled IN (0, 1)),
    -- JSON object: Telegram {"chat_id":"...","thread_id":null}
    --              Zalo     {"recipient_id":"..."}
    target_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(target_json) AND length(target_json) <= 4096),
    -- JSON array các topic đăng ký trong tập:
    -- ["reminder_due","task_completed","task_failed","schedule_triggered","wiki_review_required","system_change_request"]
    subscribed_topics_json TEXT NOT NULL DEFAULT '["reminder_due","task_completed","task_failed"]'
        CHECK (json_valid(subscribed_topics_json) AND length(subscribed_topics_json) <= 2048),
    -- Bản ghi JSON đã mã hóa AES-GCM từ CredentialCipher::seal (không bao giờ lưu plaintext token)
    encrypted_credential TEXT CHECK (encrypted_credential IS NULL OR length(encrypted_credential) <= 8192),
    updated_at_ms INTEGER NOT NULL
);

INSERT INTO notification_channel_configs (channel_kind, enabled, target_json, subscribed_topics_json, encrypted_credential, updated_at_ms)
VALUES
    ('telegram', 0, '{"chat_id":"","thread_id":null}', '["reminder_due","task_completed","task_failed"]', NULL, 0),
    ('zalo', 0, '{"recipient_id":""}', '["reminder_due","task_completed","task_failed"]', NULL, 0);

-- 2. Hộp thư thông báo mặc định cho Web UI (Notification Inbox)
CREATE TABLE notifications_inbox (
    id TEXT PRIMARY KEY,
    dedup_key TEXT UNIQUE,
    topic TEXT NOT NULL CHECK (topic IN (
        'reminder_due',
        'task_completed',
        'task_failed',
        'schedule_triggered',
        'wiki_review_required',
        'system_change_request'
    )),
    severity TEXT NOT NULL CHECK (severity IN ('info', 'success', 'warning', 'error')),
    title TEXT NOT NULL CHECK (length(title) BETWEEN 1 AND 512),
    body TEXT NOT NULL CHECK (length(body) <= 8192),
    source_kind TEXT CHECK (source_kind IS NULL OR source_kind IN ('job', 'schedule', 'wiki', 'system')),
    source_id TEXT CHECK (source_id IS NULL OR length(source_id) <= 256),
    -- JSON object chứa thông tin mở rộng (ví dụ danh sách artifacts tóm tắt: id, name, kind, download_url)
    metadata_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(metadata_json) AND length(metadata_json) <= 16384),
    is_read INTEGER NOT NULL DEFAULT 0 CHECK (is_read IN (0, 1)),
    read_at_ms INTEGER,
    created_at_ms INTEGER NOT NULL
);

CREATE INDEX idx_notifications_inbox_created ON notifications_inbox (created_at_ms DESC, id DESC);
CREATE INDEX idx_notifications_inbox_unread ON notifications_inbox (is_read, created_at_ms DESC);

-- 3. Hàng đợi chuyển phát ra kênh ngoại vi (Channel Delivery Outbox)
CREATE TABLE notification_outbox (
    id TEXT PRIMARY KEY,
    notification_id TEXT NOT NULL REFERENCES notifications_inbox(id) ON DELETE CASCADE,
    channel_kind TEXT NOT NULL CHECK (channel_kind IN ('telegram', 'zalo')),
    status TEXT NOT NULL CHECK (status IN ('pending', 'sending', 'sent', 'failed')),
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    max_attempts INTEGER NOT NULL DEFAULT 4 CHECK (max_attempts BETWEEN 1 AND 10),
    next_attempt_at_ms INTEGER NOT NULL,
    last_error_code TEXT CHECK (last_error_code IS NULL OR length(last_error_code) <= 128),
    external_message_id TEXT CHECK (external_message_id IS NULL OR length(external_message_id) <= 256),
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    UNIQUE (notification_id, channel_kind)
);

CREATE INDEX idx_notification_outbox_due ON notification_outbox (status, next_attempt_at_ms)
    WHERE status IN ('pending', 'sending');
```

---

## 4. Chi tiết Thiết kế Module Rust (`crates/voice-agent-server`)

### 4.1. Cấu trúc File trong `crates/voice-agent-server/src/services/notification_hub/`

Tuân thủ quy tắc tách module trong [`CONTEXT.md`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/CONTEXT.md#L25-L29) (code và test tách file riêng):

```text
crates/voice-agent-server/src/services/notification_hub/
├── mod.rs              # Public Interface của NotificationHub + khởi tạo worker
├── types.rs            # Strong-typed enums, NotificationContent, Settings, Patch inputs
├── store.rs            # Truy vấn SQLite (inbox, outbox, settings CAS + Admin Audit Event, retention)
├── bridge.rs           # Chuyển đổi TaskEvent, Wiki Review, System Change Request -> NotificationInput
├── dispatcher.rs       # Vòng lặp Outbox worker, exponential backoff, format nội dung tin nhắn
├── adapters.rs         # Trait OutboundChannelAdapter + TelegramBotAdapter + ZaloBotAdapter + RecordingChannelAdapter
└── tests.rs            # Unit & integration tests qua Interface của NotificationHub
```

### 4.2. Đặc tả Interface & Types (`mod.rs` & `types.rs`)

Sử dụng các enum có kiểu mạnh (`NotificationTopic`, `NotificationSeverity`, `NotificationSourceKind`, `OutboxDeliveryStatus`, `ExternalChannelKind`) và gom cụm dữ liệu dùng chung vào `NotificationContent` để tránh trùng lặp:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationTopic {
    ReminderDue,
    TaskCompleted,
    TaskFailed,
    ScheduleTriggered,
    WikiReviewRequired,
    SystemChangeRequest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationSeverity {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationSourceKind {
    Job,
    Schedule,
    Wiki,
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutboxDeliveryStatus {
    Pending,
    Sending,
    Sent,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalChannelKind {
    Telegram,
    Zalo,
}

/// Cụm nội dung cốt lõi dùng chung cho Input, Inbox View và Outbound Payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationContent {
    pub topic: NotificationTopic,
    pub severity: NotificationSeverity,
    pub title: String,
    pub body: String,
    pub source_kind: Option<NotificationSourceKind>,
    pub source_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationInput {
    pub dedup_key: Option<String>,
    #[serde(flatten)]
    pub content: NotificationContent,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelDeliveryStatusView {
    pub channel: ExternalChannelKind,
    pub status: OutboxDeliveryStatus,
    pub attempts: i64,
    pub last_error_code: Option<String>,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationItemView {
    pub id: String,
    #[serde(flatten)]
    pub content: NotificationContent,
    pub metadata: serde_json::Value,
    pub is_read: bool,
    pub read_at_ms: Option<i64>,
    pub created_at_ms: i64,
    pub deliveries: Vec<ChannelDeliveryStatusView>,
}
```

**Interface của `NotificationHub`:**

```rust
pub struct NotificationHub {
    database: Database,
    secret_resolver: Arc<dyn SecretResolver>,
    adapters: HashMap<ExternalChannelKind, Arc<dyn OutboundChannelAdapter>>,
    web_tx: broadcast::Sender<NotificationSseEvent>,
    outbox_wake: Arc<Notify>,
    workers_started: AtomicBool,
}

impl NotificationHub {
    pub fn new(
        database: Database,
        secret_resolver: Arc<dyn SecretResolver>,
        adapters: Vec<Arc<dyn OutboundChannelAdapter>>,
    ) -> Arc<Self>;

    /// Điểm vào duy nhất để phát thông báo từ mọi sự kiện trong hệ thống.
    /// 1. Lưu vào `notifications_inbox` (nếu trùng `dedup_key` thì trả về bản ghi hiện có).
    /// 2. Kiểm tra kênh `telegram` / `zalo` nào đang `enabled` và đăng ký `input.content.topic` để tạo dòng `notification_outbox`.
    /// 3. Phát sự kiện `NotificationSseEvent::Created` lên `web_tx` và gọi `outbox_wake.notify_one()`.
    pub async fn publish(&self, input: NotificationInput) -> Result<NotificationItemView, NotificationHubError>;

    /// Đăng ký stream SSE cho Admin Web UI.
    pub fn subscribe_web(&self) -> broadcast::Receiver<NotificationSseEvent>;

    /// Truy vấn danh sách Notification Inbox kèm tổng số `unread_count`.
    pub async fn list_inbox(&self, query: &InboxListQuery) -> Result<InboxPageView, NotificationHubError>;

    /// Đánh dấu đã đọc 1 danh sách ID hoặc toàn bộ (`mark_all = true`), đồng thời phát SSE `UnreadCountChanged`.
    pub async fn mark_read(&self, ids: &[String], mark_all: bool) -> Result<u64, NotificationHubError>;

    /// Xóa 1 thông báo cụ thể (`id = Some(...)`) hoặc xóa toàn bộ thông báo đã đọc (`read_only = true`).
    pub async fn delete_inbox(&self, id: Option<&str>, read_only: bool) -> Result<u64, NotificationHubError>;

    /// Đọc cấu hình kênh hiện tại (đã redact secret thành metadata + cờ `has_saved_token`, `has_env_fallback`).
    pub async fn get_settings(&self) -> Result<NotificationSettingsView, NotificationHubError>;

    /// Cập nhật cấu hình kênh với kiểm tra `If-Match: "<expected_revision>"` và ghi `Admin Audit Event`.
    /// Trường `bot_token` dùng kiểu `Patch<SecretValue>`:
    /// - `Patch::Absent`: giữ nguyên `encrypted_credential` hiện tại.
    /// - `Patch::Clear(None)`: xóa `encrypted_credential` về `NULL`.
    /// - `Patch::Clear(Some(secret))`: mã hóa qua `secret_resolver.seal(&secret, scope)`.
    pub async fn update_settings(
        &self,
        expected_revision: i64,
        update: NotificationSettingsUpdate,
        request_id: &str,
    ) -> Result<NotificationSettingsView, NotificationHubError>;

    /// Gửi thông báo thử nghiệm trực tiếp qua Adapter của kênh (`telegram` hoặc `zalo`)
    /// mà không ghi vào `notifications_inbox`. Hỗ trợ dùng token nháp trong request hoặc token đã lưu.
    pub async fn test_channel(
        &self,
        channel: ExternalChannelKind,
        input: TestChannelInput,
    ) -> Result<TestChannelOutput, NotificationHubError>;

    /// Khởi chạy idempotent (`workers_started.swap(true, Ordering::AcqRel)`) cho background worker
    /// xử lý `notification_outbox` và dọn dẹp theo `retention_days`, dừng sạch theo `RuntimeLifecycle::stopping()`.
    pub fn start_workers(self: &Arc<Self>, stopping: CancellationToken);
}
```

### 4.3. Đặc tả Internal Seam & Adapters (`adapters.rs`)

> **Bảo toàn bất biến của `SecretValue`**: Theo [`CONTEXT.md`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/CONTEXT.md#L450-L452) và [`secrets.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/database/security/secrets.rs#L74-L92), `SecretValue` bọc `Zeroizing<String>` và **không** cài đặt `Clone`. Do đó `ResolvedChannelTarget` chỉ derive `Debug` (không derive `Clone`) và được truyền theo tham chiếu `&ResolvedChannelTarget`.

```rust
#[derive(Debug)]
pub struct ResolvedChannelTarget {
    pub token: SecretValue,
    pub recipient_id: String, // chat_id cho Telegram, recipient_id (chat_id) cho Zalo Bot
    pub thread_id: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct OutboundMessagePayload {
    pub notification_id: String,
    pub content: NotificationContent,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExternalDeliveryReceipt {
    pub external_message_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelDeliveryError {
    InvalidConfig(&'static str),
    Unauthorized,             // HTTP 401/403 -> Không retry (permanent failure)
    BadRequest,               // HTTP 400/404 -> Không retry (permanent failure)
    RateLimited(Option<u64>), // HTTP 429 -> Retry
    TransientNetwork,         // Timeout / 5xx -> Retry
}

#[async_trait::async_trait]
pub trait OutboundChannelAdapter: Send + Sync {
    fn channel_kind(&self) -> ExternalChannelKind;
    async fn send(
        &self,
        target: &ResolvedChannelTarget,
        payload: &OutboundMessagePayload,
    ) -> Result<ExternalDeliveryReceipt, ChannelDeliveryError>;
}
```

#### Chi tiết `TelegramBotAdapter`:
- **Endpoint cố định**: `https://api.telegram.org` (chỉ có hàm `#[cfg(test)] pub fn with_base_url_for_test(...)` trong unit test; Admin API không thể thay đổi host để chống đánh cắp token).
- **Request**: `POST {base_url}/bot{token.expose()}/sendMessage` với JSON:
  ```json
  {
    "chat_id": "<target.recipient_id>",
    "message_thread_id": 123,
    "text": "🔔 [Nhắc nhở] Họp nhóm lúc 15:00\nNội dung chi tiết...",
    "disable_web_page_preview": true
  }
  ```
- **Timeout**: `10s` cứng trên `reqwest::Client` dùng chung (`redirect::Policy::none()`).
- **Xử lý lỗi**:
  - `200 OK` với `{"ok": true, "result": {"message_id": 42}}` $\rightarrow$ `Ok(ExternalDeliveryReceipt { external_message_id: Some("42") })`.
  - `401` / `403` $\rightarrow$ `Err(ChannelDeliveryError::Unauthorized)` (chuyển thẳng sang `OutboxDeliveryStatus::Failed`).
  - `400` / `404` $\rightarrow$ `Err(ChannelDeliveryError::BadRequest)` (chuyển thẳng sang `OutboxDeliveryStatus::Failed`).
  - `429` $\rightarrow$ Đọc `parameters.retry_after` (nếu có) $\rightarrow$ `Err(ChannelDeliveryError::RateLimited(retry_after))`.
  - `5xx` hoặc lỗi kết nối/timeout $\rightarrow$ `Err(ChannelDeliveryError::TransientNetwork)`.

#### Chi tiết `ZaloBotAdapter`:
- **Endpoint cố định**: `https://bot-api.zapps.me` (Zalo Bot Platform chuẩn; không trộn lẫn giao thức Zalo OA v3 để tránh *Divergent Change*).
- **Request**: `POST {base_url}/bot{token.expose()}/sendMessage` với JSON:
  ```json
  {
    "chat_id": "<target.recipient_id>",
    "text": "🔔 [Nhắc nhở] Họp nhóm lúc 15:00\nNội dung chi tiết..."
  }
  ```
- **Không lộ Secret**: Tuyệt đối không ghi URL đầy đủ (vì chứa `/bot{token}`) vào log hay bảng `notification_outbox.last_error_code`. Chỉ lưu mã lỗi chuẩn (`"unauthorized"`, `"rate_limited"`, `"invalid_recipient"`, `"upstream_timeout"`, `"upstream_error"`).

### 4.4. Quản lý Bảo mật Token (`SecretResolver`), `Patch<SecretValue>` & `Admin Audit Event`

1. **Dẫn xuất biến môi trường fallback** trong [`crates/voice-agent-server/src/database/security/secrets.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/database/security/secrets.rs):
   ```rust
   pub fn notification_channel_secret_env(channel: &str) -> Option<&'static str> {
       match channel {
           "telegram" => Some("VOICE_NOTIFICATION_TELEGRAM_BOT_TOKEN"),
           "zalo" => Some("VOICE_NOTIFICATION_ZALO_BOT_TOKEN"),
           _ => None,
       }
   }
   ```
2. **Xử lý `Patch<SecretValue>` và mã hóa khi `update_settings`**:
   - Tái sử dụng kiểu [`Patch<T>`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/app/admin/mod.rs#L628-L642) (nâng khả năng hiển thị lên `pub(crate) enum Patch<T>`):
     - `Patch::Absent`: Giữ nguyên cột `encrypted_credential` hiện có trong DB.
     - `Patch::Clear(None)`: Đặt cột `encrypted_credential = NULL` (nếu kênh vẫn bật mà không có env fallback thì trả `400 Bad Request` với mã `"missing_bot_token"`).
     - `Patch::Clear(Some(secret))`: Gọi `secret_resolver.seal(&secret, scope)` với `scope = "notification_channel:telegram"` hoặc `"notification_channel:zalo"`:
       - Nếu trả `Err(SecretResolveError::Invalid)` (token rỗng, `> 4096` bytes hoặc chứa ký tự không phải ASCII graphic theo [`credentials::valid_input`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/database/security/credentials.rs#L30-L33)) $\rightarrow$ trả `400 Bad Request` (`"invalid_bot_token"`).
       - Nếu trả `Err(SecretResolveError::Unavailable)` (môi trường chưa cấu hình `VOICE_CREDENTIAL_KEY_1` hoặc `credential_keys_file`) $\rightarrow$ trả `503 Service Unavailable` (`"credential_encryption_unavailable"`).
3. **Ghi `Admin Audit Event`**:
   - Trong cùng transaction SQLite cập nhật `notification_settings_state`, gọi hàm [`audit`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/database/mod.rs#L24) (`resource_type = "notification_settings"`, `resource_key = "global"`, `action = "update"`, `outcome = AuditOutcome::Applied` hoặc `AuditOutcome::Conflict` khi lệch `If-Match`) mà không ghi kèm bất kỳ giá trị token nào.
4. **Giải mã tại thời điểm gửi (`dispatcher.rs` & `test_channel`)**:
   - Tạo chuỗi tham chiếu qua [`credentials::reference`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/database/security/credentials.rs#L38-L41):
     ```rust
     let secret_ref_str = credentials::reference(
         scope,
         row.encrypted_credential.as_deref(),
         notification_channel_secret_env(channel_str).map(str::to_owned),
     );
     ```
   - Giải mã bằng `secret_resolver.resolve(&SecretRef::parse(secret_ref_str)?)` ngay trước khi gọi `adapter.send(&target, &payload)`.

### 4.5. Đấu nối Toàn bộ 6 Chủ đề Sự kiện (`bridge.rs`, `reminder_scheduler.rs`, `app/mod.rs`, `knowledge_processing.rs`, `system_mcp/`)

1. **Khởi tạo `NotificationHub` và Worker trong [`router_with_state`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/app/mod.rs#L104-L125)**:
   - Tại `AppState::build_from_config_with` ([`app/state.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/app/state.rs#L168-L195)) hoặc `router_with_state`, khi `state.database` tồn tại, khởi tạo `NotificationHub::new(db.clone(), state.secret_resolver.clone(), default_adapters())` và gọi `hub.start_workers(state.lifecycle.stopping().clone())` (tương tự cơ chế `state.reminder_scheduler_started` tại [`app/mod.rs#L105-L115`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/app/mod.rs#L105-L115)).
2. **Nguồn 1 (`ReminderDue` từ Device Reminders) — [`services::reminder_scheduler::start`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/services/notifications/reminder_scheduler.rs#L18-L90)**:
   - Truyền thêm `notification_hub: Option<Arc<NotificationHub>>` vào `services::reminder_scheduler::start`.
   - Khi `claim_next_due_for_device` trả về `Some(due)` ([`reminder_scheduler.rs#L51`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/services/notifications/reminder_scheduler.rs#L51)), bên cạnh việc gửi `SessionMessage::DeliverReminder` xuống phiên WebSocket của thiết bị, gọi `hub.publish(bridge::from_due_device_reminder(&due))`:
     - `dedup_key`: `Some(format!("device_reminder_due:{}", due.reminder_id))` (đảm bảo nếu thiết bị retry playback nhiều lần cho cùng `reminder_id`, hộp thư Web UI và Telegram/Zalo chỉ nhận đúng 1 thông báo).
     - `topic`: `NotificationTopic::ReminderDue`, `severity`: `NotificationSeverity::Warning`
     - `title`: `"Nhắc nhở đến hạn"`
     - `body`: `due.message.clone()`
     - `source_kind`: `Some(NotificationSourceKind::Schedule)`, `source_id`: `Some(due.reminder_id.clone())`
     - `metadata`: `json!({"device_row_id": due.device_row_id, "resource_href": "/devices"})`
3. **Nguồn 2 (`ReminderDue`, `TaskCompleted`, `TaskFailed`, `ScheduleTriggered`, `WikiReviewRequired` từ `TaskManager`) — `prepare_background` trong [`crates/voice-agent-server/src/app/mod.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/app/mod.rs#L434-L487)**:
   - Hiện tại vòng lặp `while let Ok(item) = sub.recv().await` ([`app/mod.rs#L451-L473`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/app/mod.rs#L451-L473)) chỉ ghi `tracing::debug!` cho `TaskEvent::ReminderDue` và ghi `tracing::warn!("task_feed_resync_required")` cho `TaskFeedItem::ResyncRequired`.
   - Nâng cấp vòng lặp để chuyển mọi `TaskFeedItem::Event(event)` qua `bridge::from_task_event(&event)` và gọi `notification_hub.publish(...)`:
     - Khi nhận [`TaskEvent::ReminderDue { schedule }`](file:///Users/lamphuchai/Desktop/project_rust/orchestrator_task_rs/src/types/event.rs#L57-L59):
       - `dedup_key`: `Some(format!("task_reminder_due:{}:{}", schedule.id, schedule.last_run_at.unwrap_or(schedule.updated_at)))`
       - `topic`: `NotificationTopic::ReminderDue`, `severity`: `NotificationSeverity::Warning`
       - `title`: `format!("Nhắc nhở: {}", schedule.title)`
       - `body`: `schedule.notes.clone().unwrap_or_else(|| schedule.title.clone())`
       - `source_kind`: `Some(NotificationSourceKind::Schedule)`, `source_id`: `Some(schedule.id)`
     - Khi nhận [`TaskEvent::TaskTerminal { task }`](file:///Users/lamphuchai/Desktop/project_rust/orchestrator_task_rs/src/types/event.rs#L40-L42):
       - Chỉ phát khi `task.status` là `TaskStatus::Completed` hoặc `TaskStatus::Failed`.
       - Lấy nội dung tóm tắt từ [`TaskView`](file:///Users/lamphuchai/Desktop/project_rust/orchestrator_task_rs/src/types/task.rs#L139-L163):
         - Với `TaskStatus::Completed`:
           - Nếu tác vụ thuộc `WikiAutoFlowBinding` (`task.flow_id == "wiki"` hoặc `task.agent_id == "wiki_root"` được đăng ký tại [`services/task_flow/mod.rs#L226-L230`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/services/task_flow/mod.rs#L226-L230)): phân loại sang `NotificationTopic::WikiReviewRequired`, `severity: NotificationSeverity::Info`, `source_kind: Some(NotificationSourceKind::Wiki)`, `source_id: Some(task.id)`, `metadata: json!({"resource_href": "/knowledge"})`.
           - Các tác vụ nền khác: `NotificationTopic::TaskCompleted`, `severity: NotificationSeverity::Success`, `body`: `task.output_preview.as_deref().or(task.progress_summary.as_deref()).unwrap_or("Tác vụ đã hoàn tất.")` (giới hạn 1000 ký tự), `source_kind: Some(NotificationSourceKind::Job)`, `source_id: Some(task.id)`.
         - Với `TaskStatus::Failed`: `NotificationTopic::TaskFailed`, `severity: NotificationSeverity::Error`, `body`: `task.error_message.as_deref().or(task.progress_summary.as_deref()).unwrap_or("Tác vụ kết thúc với lỗi.")`, `source_kind: Some(NotificationSourceKind::Job)`, `source_id: Some(task.id)`.
     - Khi nhận [`TaskEvent::ScheduleTriggered { schedule, spawned_task_id }`](file:///Users/lamphuchai/Desktop/project_rust/orchestrator_task_rs/src/types/event.rs#L60-L63):
       - `dedup_key`: `Some(format!("schedule_triggered:{}:{}", schedule.id, spawned_task_id))`
       - `topic`: `NotificationTopic::ScheduleTriggered`, `severity`: `NotificationSeverity::Info`
       - `title`: `format!("Lịch định kỳ đã khởi chạy: {}", schedule.title)`
       - `source_kind`: `Some(NotificationSourceKind::Job)`, `source_id`: `Some(spawned_task_id)`
     - Khi gặp `TaskFeedItem::ResyncRequired`: giữ nguyên hành vi hiện có tại [`app/mod.rs#L465-L468`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/app/mod.rs#L465-L468) (log `task_feed_resync_required` và tiếp tục nhận sự kiện trên `sub` mà không phá vỡ vòng lặp).
4. **Nguồn 3 (`WikiReviewRequired` từ luồng Tri thức / Wiki) — [`app/admin/knowledge_processing.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/app/admin/knowledge_processing.rs#L27-L61) & [`app/admin/knowledge/mod.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/app/admin/knowledge/mod.rs#L48-L62)**:
   - Kiến trúc Knowledge trên server hiện tách thành 2 cụm:
     - [`app/admin/knowledge/mod.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/app/admin/knowledge/mod.rs) + [`raw.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/app/admin/knowledge/raw.rs): quản lý cây thư mục vật lý (`sources/`, `wiki/`) và gọi [`trigger_source_analysis`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/app/admin/knowledge/mod.rs#L48-L62) $\rightarrow$ ủy quyền cho [`services::task_flow::submit_wiki_source_analysis`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/services/task_flow/mod.rs#L240-L274) (khi hoàn tất sẽ phát thông báo qua `TaskEvent::TaskTerminal` ở mục 3).
     - [`app/admin/knowledge_processing.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/app/admin/knowledge_processing.rs#L27-L61): khi `lint_wiki` phát hiện `report.status == "needs_attention"` (tồn tại `broken_links`, `orphan_pages` hoặc `staged_drafts` chờ duyệt), phát `NotificationTopic::WikiReviewRequired` với `source_kind: Some(NotificationSourceKind::Wiki)` và `dedup_key` băm theo danh sách vấn đề để tránh thông báo trùng.
5. **Nguồn 4 (`SystemChangeRequest` từ `SystemMcp`) — [`tools/system_mcp/mod.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/tools/system_mcp/mod.rs#L90-L115) & [`proposals.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/tools/system_mcp/proposals.rs#L88-L173)**:
   - Bổ sung trường `notification_hub: Option<Arc<NotificationHub>>` và builder `.with_notification_hub(state.notification_hub.clone())` vào `SystemMcp` (gắn tại [`router_with_state` trong `app/mod.rs#L116-L124`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/app/mod.rs#L116-L124)).
   - Trong [`SystemMcp::propose`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/tools/system_mcp/proposals.rs#L157-L173), ngay khi thêm `ChangeRequest` (`mcp_request_server_change`, `mcp_request_agent_binding_change`, `mcp_request_tool_review`) vào `self.pending`, phát bất đồng bộ qua `notification_hub.publish(...)` với `dedup_key: Some(format!("system_change_request:{}", view.id))`, `topic: NotificationTopic::SystemChangeRequest`, `severity: NotificationSeverity::Warning`, `source_kind: Some(NotificationSourceKind::System)`, `source_id: Some(view.id.clone())`.

---

## 5. Đặc tả HTTP Contract (Admin API)

Tất cả các route mới được gắn vào [`crates/voice-agent-server/src/app/admin/mod.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/app/admin/mod.rs) dưới lớp xác thực Bearer Token hiện hành (`authenticate` middleware), triển khai handler trong `crates/voice-agent-server/src/app/admin/notification_hub.rs`.

### 5.1. Danh sách Endpoints

| Method & Path | Mô tả |
| :--- | :--- |
| `GET /api/admin/notifications` | Lấy danh sách thông báo trong **Notification Inbox** (`?page=1&page_size=30&unread_only=false&topic=...`), trả kèm `unread_count`. |
| `POST /api/admin/notifications/read` | Đánh dấu đã đọc (`{"ids": ["..."], "all": false}`). Trả về `{"updated": N, "unread_count": M}`. |
| `DELETE /api/admin/notifications/{id}` | Xóa 1 thông báo khỏi hộp thư Web UI (gọi `hub.delete_inbox(Some(&id), false)`). |
| `POST /api/admin/notifications/purge` | Xóa các thông báo đã đọc (`{"read_only": true}`, gọi `hub.delete_inbox(None, true)`). |
| `GET /api/admin/notifications/stream` | SSE stream thời gian thực (`event: ready`, `event: notification_created`, `event: unread_count_changed`, `event: resync_required`). |
| `GET /api/admin/notifications/settings` | Đọc cấu hình kênh Web UI, Telegram, Zalo (kèm `revision`, `credential_metadata`, `has_saved_token`, `has_env_fallback`). |
| `PUT /api/admin/notifications/settings` | Cập nhật cấu hình kênh (yêu cầu header `If-Match: "<revision>"`). |
| `POST /api/admin/notifications/channels/{channel}/test` | Gửi tin nhắn thử nghiệm tới `telegram` hoặc `zalo` (dùng token nháp gửi trong body hoặc token đã lưu). |

### 5.2. Wire DTOs (`apps/admin-web/src/api/types/notifications.ts`)

```ts
export type NotificationTopic =
  | 'reminder_due'
  | 'task_completed'
  | 'task_failed'
  | 'schedule_triggered'
  | 'wiki_review_required'
  | 'system_change_request'

export type NotificationSeverity = 'info' | 'success' | 'warning' | 'error'
export type NotificationSourceKind = 'job' | 'schedule' | 'wiki' | 'system'
export type ExternalChannelKind = 'telegram' | 'zalo'
export type OutboxDeliveryStatus = 'pending' | 'sending' | 'sent' | 'failed'

export interface ChannelDeliveryStatus {
  channel: ExternalChannelKind
  status: OutboxDeliveryStatus
  attempts: number
  last_error_code: string | null
  updated_at_ms: number
}

export interface NotificationItem {
  id: string
  topic: NotificationTopic
  severity: NotificationSeverity
  title: string
  body: string
  source_kind: NotificationSourceKind | null
  source_id: string | null
  metadata: Record<string, unknown>
  is_read: boolean
  read_at_ms: number | null
  created_at_ms: number
  deliveries: ChannelDeliveryStatus[]
}

export interface NotificationInboxPage {
  items: NotificationItem[]
  total: number
  unread_count: number
  page: number
  page_size: number
}

export interface ChannelCredentialMetadata {
  id: string
  masked_key: string
  key_version: number
  status: string
}

export interface TelegramChannelSettings {
  enabled: boolean
  chat_id: string
  thread_id: number | null
  subscribed_topics: NotificationTopic[]
  has_saved_token: boolean
  has_env_fallback: boolean
  credential_metadata: ChannelCredentialMetadata | null
}

export interface ZaloChannelSettings {
  enabled: boolean
  recipient_id: string
  subscribed_topics: NotificationTopic[]
  has_saved_token: boolean
  has_env_fallback: boolean
  credential_metadata: ChannelCredentialMetadata | null
}

export interface NotificationSettings {
  revision: number
  web_ui: {
    enabled: true // Luôn là true (default channel)
    toast_enabled: boolean
    retention_days: number
  }
  telegram: TelegramChannelSettings
  zalo: ZaloChannelSettings
}

export interface NotificationSettingsUpdate {
  web_ui: {
    toast_enabled: boolean
    retention_days: number
  }
  telegram: {
    enabled: boolean
    chat_id: string
    thread_id?: number | null
    subscribed_topics: NotificationTopic[]
    /**
     * Khớp ngữ nghĩa Patch<SecretValue> của backend:
     * - undefined (absent): giữ nguyên token đã lưu
     * - null: xóa token đã lưu trong DB
     * - string: mã hóa và lưu token mới
     */
    bot_token?: string | null
  }
  zalo: {
    enabled: boolean
    recipient_id: string
    subscribed_topics: NotificationTopic[]
    bot_token?: string | null
  }
}
```

---

## 6. Thiết kế Giao diện Admin Web (`apps/admin-web`)

### 6.1. Chuông Thông báo Toàn cục (`NotificationBell.vue`) trên Mobile & Desktop

Trong [`AdminShell.vue`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/apps/admin-web/src/layouts/AdminShell.vue#L65), phần tử `<header class="... lg:hidden">` chỉ hiển thị trên màn hình nhỏ (`< 1024px`), còn trên màn hình Desktop (`>= 1024px`) thanh điều hướng nằm trong [`SidebarContent.vue`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/apps/admin-web/src/components/SidebarContent.vue#L61-L72) (hỗ trợ cả trạng thái mở rộng `w-64` và thu gọn `w-[72px]`).

Do đó, `NotificationBell.vue` nhận prop `compact?: boolean` và được đặt tại **2 vị trí phối hợp**:
1. **Trên Mobile (`< lg`)**: Đặt cạnh `ThemeToggle` trên thanh `<header class="... lg:hidden">` của [`AdminShell.vue`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/apps/admin-web/src/layouts/AdminShell.vue#L72).
2. **Trên Desktop (`>= lg`)**: Đặt trong khu vực tiện ích chân sidebar (cạnh `ThemeToggle`) của [`SidebarContent.vue`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/apps/admin-web/src/components/SidebarContent.vue#L61-L72):
   - Khi `collapsed === true`: hiển thị nút icon `Bell` kèm chấm/số badge đỏ `unread_count`.
   - Khi `collapsed === false`: hiển thị hàng đầy đủ gồm icon `Bell`, nhãn *"Thông báo"* và badge `unread_count`.
3. **Popover Inbox & Toast Stack**:
   - Bấm vào `NotificationBell.vue` mở Popover hiển thị danh sách thông báo (sử dụng `position: fixed` neo động theo tọa độ nút chuông để không bị cắt bởi `overflow-hidden` của `SidebarContent.vue`, đồng bộ hệ thống studio design tokens `bg-card`, `studio-panel`, `admin-input`), bộ lọc **Tất cả / Chưa đọc**, nút **Đánh dấu đã đọc tất cả**, trạng thái chuyển phát (`TG ✓`, `Zalo ✓`, `TG !`), và điều hướng thẳng tới `/jobs/:source_id` hoặc `/knowledge`.
   - Khi SSE `/api/admin/notifications/stream` bắn `notification_created` và `web_ui.toast_enabled === true`, hiển thị Toast góc dưới phải màn hình trong 6 giây.

### 6.2. Khu vực Thiết lập Kênh Thông báo (`NotificationSettingsCard.vue`)

Component `apps/admin-web/src/components/notifications/NotificationSettingsCard.vue` được nhúng vào trang Cấu hình Nền ([`BackgroundPage.vue`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/apps/admin-web/src/pages/background/BackgroundPage.vue)):

```text
┌────────────────────────────────────────────────────────────────────────────┐
│ Kênh Thông báo (Notification Channels)                      [Lưu thay đổi] │
├────────────────────────────────────────────────────────────────────────────┤
│ 1. Web UI (Mặc định — Luôn bật)                                            │
│    [x] Hiển thị popup Toast khi có thông báo mới                           │
│    Lưu trữ lịch sử hộp thư: [ 30 ] ngày                                    │
├────────────────────────────────────────────────────────────────────────────┤
│ 2. Telegram Bot                                           [ Bật / Tắt ]    │
│    Bot Token: [ …7a9f (Đã lưu) / Nhập token mới... ]   [Xóa / Hoàn tác]    │
│    Chat ID:   [ -100123456789 ]   Thread ID (tùy chọn): [ trống ]          │
│    Sự kiện nhận: [x] Lời nhắc đến hạn  [x] Task hoàn tất  [x] Task lỗi     │
│                  [ ] Lịch định kỳ chạy [x] Cần duyệt Wiki [x] Duyệt MCP    │
│                                                  [Gửi tin nhắn thử nghiệm] │
├────────────────────────────────────────────────────────────────────────────┤
│ 3. Zalo Bot                                               [ Bật / Tắt ]    │
│    Bot Token: [ …3c1b (Đã lưu) / Nhập token mới... ]   [Xóa / Hoàn tác]    │
│    Recipient / Chat ID: [ 84912345678 ]                                    │
│    Sự kiện nhận: [x] Lời nhắc đến hạn  [x] Task hoàn tất  [x] Task lỗi     │
│                  [ ] Lịch định kỳ chạy [x] Cần duyệt Wiki [x] Duyệt MCP    │
│                                                  [Gửi tin nhắn thử nghiệm] │
└────────────────────────────────────────────────────────────────────────────┘
```

### 6.3. Cơ chế Dữ liệu Demo Có Trạng thái (`Stateful Demo Store Fallback`)

Trong giai đoạn phía Rust server chưa triển khai các route `/api/admin/notifications*`, [`notificationsApi`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/apps/admin-web/src/api/notifications.ts#L351-L580) tích hợp sẵn tầng **Stateful Demo Store** trong bộ nhớ để kiểm thử đầy đủ luồng UI:
- **Điều kiện kích hoạt an toàn (`shouldFallbackToDemo`)**: Chỉ chuyển sang chế độ Demo khi server trả về `404 Not Found` / `501 Not Implemented` hoặc khi mạng offline (server chưa bật). Nếu backend đã bật và trả về lỗi xác thực/nghiệp vụ thật (`400`, `401`, `403`, `409 Conflict` do lệch `If-Match`, `500`), lỗi được ném nguyên vẹn lên UI mà không bị che giấu.
- **Trải nghiệm tương tác đầy đủ**:
  - Hộp thư khởi tạo sẵn 6 thông báo mẫu thuộc đủ 6 `NotificationTopic` kèm trạng thái chuyển phát (`sent`, `failed`) của `telegram` / `zalo`.
  - Hỗ trợ lọc **Tất cả / Chưa đọc**, đánh dấu đã đọc từng mục hoặc toàn bộ, xóa từng thông báo, dọn sạch thông báo đã đọc (`purgeRead`), và nút **Tạo thông báo demo (`simulateDemoNotification`)** phát sự kiện trực tiếp qua `subscribeNotificationEvents` để kích hoạt **Live Toast** (6 giây).
  - Hỗ trợ lưu cấu hình kênh (`updateSettings`) với đúng ngữ nghĩa `Patch<SecretValue>` cho `bot_token` (giữ nguyên khi để trống `undefined`, xóa khi `null`, hoặc sinh `masked_key` mới khi nhập chuỗi token) và kiểm tra gửi thử (`testChannel`) cho cả `telegram` và `zalo`.
  - [`NotificationSettingsCard.vue`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/apps/admin-web/src/components/notifications/NotificationSettingsCard.vue) được đặt độc lập với trạng thái tải `/api/admin/background/settings` trong [`BackgroundPage.vue`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/apps/admin-web/src/pages/background/BackgroundPage.vue#L579) để luôn hiển thị và thao tác được ngay cả khi backend chưa chạy.

---

## 7. Danh sách File Thay đổi & Tạo mới Khi Triển khai

### 7.1. Backend (`crates/voice-agent-server`) — `✅ ĐÃ HOÀN THÀNH`

| File | Trạng thái | Hành động |
| :--- | :--- | :--- |
| [`crates/voice-agent-server/migrations/0137_notification_hub.sql`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/migrations/0137_notification_hub.sql) | ✅ Đã xong | **NEW**: Tạo bảng `notification_settings_state`, `notification_channel_configs`, `notifications_inbox`, `notification_outbox`. |
| [`crates/voice-agent-server/src/database/security/secrets.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/database/security/secrets.rs) | ✅ Đã xong | Cập nhật: thêm `notification_channel_secret_env("telegram" \| "zalo")`. |
| [`crates/voice-agent-server/src/services/notification_hub/mod.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/services/notification_hub/mod.rs) | ✅ Đã xong | **NEW**: Định nghĩa `NotificationHub` (`workers_started: AtomicBool`) và public interface. |
| [`crates/voice-agent-server/src/services/notification_hub/types.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/services/notification_hub/types.rs) | ✅ Đã xong | **NEW**: Định nghĩa strong-typed enums, `NotificationContent`, DTOs và `Patch<SecretValue>` inputs. |
| [`crates/voice-agent-server/src/services/notification_hub/store.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/services/notification_hub/store.rs) | ✅ Đã xong | **NEW**: Thực thi SQLite queries cho Inbox, Outbox, CAS Settings + `Admin Audit Event` và Retention. |
| [`crates/voice-agent-server/src/services/notification_hub/adapters.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/services/notification_hub/adapters.rs) | ✅ Đã xong | **NEW**: Định nghĩa internal seam `OutboundChannelAdapter`, `TelegramBotAdapter`, `ZaloBotAdapter`, `RecordingChannelAdapter`. |
| [`crates/voice-agent-server/src/services/notification_hub/dispatcher.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/services/notification_hub/dispatcher.rs) | ✅ Đã xong | **NEW**: Outbox worker loop, retry backoff, message formatting. |
| [`crates/voice-agent-server/src/services/notification_hub/bridge.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/services/notification_hub/bridge.rs) | ✅ Đã xong | **NEW**: Chuyển đổi `DueReminder` (từ `reminder_scheduler`), `TaskEvent` (gồm cả `WikiAutoFlowBinding` `wiki_root`), Wiki Lint và System Change Request sang `NotificationInput`. |
| [`crates/voice-agent-server/src/services/notification_hub/tests.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/services/notification_hub/tests.rs) | ✅ Đã xong | **NEW**: Unit & integration tests cho `NotificationHub` và Admin HTTP endpoints. |
| [`crates/voice-agent-server/src/services/mod.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/services/mod.rs) | ✅ Đã xong | Export `pub mod notification_hub;`. |
| [`crates/voice-agent-server/src/services/notifications/reminder_scheduler.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/services/notifications/reminder_scheduler.rs#L18-L90) | ✅ Đã xong | Cập nhật: thêm `start_with_hub` để phát `NotificationTopic::ReminderDue` khi `claim_next_due_for_device` trả về `Some(due)`. |
| [`crates/voice-agent-server/src/app/state.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/app/state.rs#L65-L90) | ✅ Đã xong | Thêm `pub notification_hub: Option<Arc<NotificationHub>>` vào `AppState`. |
| [`crates/voice-agent-server/src/app/mod.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/app/mod.rs#L104-L130) | ✅ Đã xong | Khởi chạy `hub.start_workers(state.lifecycle.stopping().clone())`, truyền `state.notification_hub` vào `reminder_scheduler::start_with_hub`, `SystemMcp` và vòng lặp `prepare_background`. |
| [`crates/voice-agent-server/src/app/admin/knowledge_processing.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/app/admin/knowledge_processing.rs#L50-L82) | ✅ Đã xong | Phát `NotificationTopic::WikiReviewRequired` qua `state.notification_hub` khi `lint_wiki` báo `!lint.is_healthy`. |
| [`crates/voice-agent-server/src/tools/system_mcp/mod.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/tools/system_mcp/mod.rs#L90-L125) & [`proposals.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/tools/system_mcp/proposals.rs#L157-L200) | ✅ Đã xong | Thêm `.with_notification_hub(...)` và phát `NotificationTopic::SystemChangeRequest` trong `SystemMcp::propose` khi tạo yêu cầu thay đổi mới. |
| [`crates/voice-agent-server/src/app/admin/notification_hub.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/app/admin/notification_hub.rs) | ✅ Đã xong | **NEW**: HTTP handlers cho `/api/admin/notifications*`. |
| [`crates/voice-agent-server/src/app/admin/mod.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/app/admin/mod.rs#L140-L162) | ✅ Đã xong | Nâng `Patch<T>` và mount các route `/notifications*` vào `admin::router`. |

### 7.2. Frontend (`apps/admin-web`) — `✅ ĐÃ HOÀN THÀNH`

| File | Trạng thái | Hành động |
| :--- | :--- | :--- |
| [`apps/admin-web/src/api/types/notifications.ts`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/apps/admin-web/src/api/types/notifications.ts) | ✅ Đã xong | **NEW**: TypeScript interfaces cho Inbox, Settings, `ChannelCredentialMetadata`, SSE events, Test channel. |
| [`apps/admin-web/src/api/notifications.ts`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/apps/admin-web/src/api/notifications.ts) | ✅ Đã xong | **NEW**: Client REST + SSE stream `listenNotificationStream` + `Stateful Demo Store` fallback. |
| [`apps/admin-web/src/api/notifications.test.ts`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/apps/admin-web/src/api/notifications.test.ts) | ✅ Đã xong | **NEW**: Unit tests cho REST endpoints, `If-Match`, lỗi `409 Conflict`, SSE stream và Demo Store fallback. |
| [`apps/admin-web/src/components/notifications/NotificationBell.vue`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/apps/admin-web/src/components/notifications/NotificationBell.vue) | ✅ Đã xong | **NEW**: Chuông thông báo (hỗ trợ cả `compact` và đầy đủ) + Popover Inbox + nút tạo thông báo demo + Toast 6 giây (chống trùng lặp giữa nhiều instance). |
| [`apps/admin-web/src/components/notifications/NotificationBell.test.ts`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/apps/admin-web/src/components/notifications/NotificationBell.test.ts) | ✅ Đã xong | **NEW**: Component tests cho badge chưa đọc, bộ lọc, điều hướng `/jobs/:source_id`, đánh dấu đã đọc, xóa và mô phỏng Toast. |
| [`apps/admin-web/src/components/notifications/NotificationSettingsCard.vue`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/apps/admin-web/src/components/notifications/NotificationSettingsCard.vue) | ✅ Đã xong | **NEW**: Form cấu hình kênh Web UI, Telegram Bot, Zalo Bot + nút Clear/Undo secret (`Patch<SecretValue>`) + nút gửi thử. |
| [`apps/admin-web/src/components/notifications/NotificationSettingsCard.test.ts`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/apps/admin-web/src/components/notifications/NotificationSettingsCard.test.ts) | ✅ Đã xong | **NEW**: Component tests cho `Patch<SecretValue>` (`undefined`, `null`, `string`), hoàn tác xóa token và gửi tin nhắn thử. |
| [`apps/admin-web/src/layouts/AdminShell.vue`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/apps/admin-web/src/layouts/AdminShell.vue) | ✅ Đã xong | Tích hợp `NotificationBell` vào thanh `<header>` mobile (`lg:hidden`). |
| [`apps/admin-web/src/components/SidebarContent.vue`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/apps/admin-web/src/components/SidebarContent.vue) | ✅ Đã xong | Tích hợp `NotificationBell` vào khu vực tiện ích chân sidebar cho màn hình Desktop (cả `collapsed` và expanded). |
| [`apps/admin-web/src/pages/background/BackgroundPage.vue`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/apps/admin-web/src/pages/background/BackgroundPage.vue) | ✅ Đã xong | Hiển thị `NotificationSettingsCard.vue` trong trang cấu hình Background (`/jobs/settings`), hoạt động cả khi server chưa bật. |
| [`apps/admin-web/src/i18n/messages.ts`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/apps/admin-web/src/i18n/messages.ts) | ✅ Đã xong | Bổ sung từ khóa đa ngôn ngữ (`vi` / `en`) cho Notification Hub. |

### 7.3. Tài liệu Domain (`GLOSSARY.md` & `CONTEXT.md`)

Khi bắt đầu triển khai code:
1. Bổ sung mục `## Multi-Channel Notifications` vào [`GLOSSARY.md`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/GLOSSARY.md) với đúng 5 định nghĩa thuần domain nêu tại mục **§1.1**.
2. Bổ sung mục `## Multi-Channel Notifications` tương ứng vào [`CONTEXT.md`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/CONTEXT.md) dẫn chiếu sang `GLOSSARY.md#multi-channel-notifications` theo đúng quy ước tại [`docs/agents/domain.md`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/docs/agents/domain.md).

---

## 8. Kế hoạch Kiểm thử & Xác minh (Verification Policy)

Tuân thủ nghiêm ngặt quy định tại [`AGENTS.md`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/AGENTS.md): **luôn sử dụng `scripts/agent-test.sh`**, chạy targeted tests trước khi chạy toàn bộ workspace, không gọi `cargo test` trực tiếp.

### 8.1. Các kịch bản kiểm thử bắt buộc tại Interface của `NotificationHub`
1. **Default Web UI Inbox & Deduplication**:
   - Gọi `hub.publish(...)` hai lần với cùng `dedup_key` $\rightarrow$ chỉ tạo 1 bản ghi trong `notifications_inbox`, `unread_count` tăng đúng 1, và phát đúng 1 sự kiện SSE.
   - Gọi `mark_read` $\rightarrow$ `is_read = true`, `unread_count` giảm về 0.
2. **Topic Routing & Multi-Channel Isolation**:
   - Bật kênh `telegram` chỉ đăng ký `["reminder_due"]`, tắt kênh `zalo`.
   - Phát sự kiện `TaskCompleted` $\rightarrow$ lưu vào Web UI Inbox nhưng **không** sinh dòng nào trong `notification_outbox`.
   - Phát sự kiện `ReminderDue` $\rightarrow$ lưu vào Web UI Inbox **và** sinh đúng 1 dòng `notification_outbox` cho `telegram`, sau đó `RecordingChannelAdapter` nhận được đúng thông điệp.
3. **Fault Isolation & Retry Backoff**:
   - Cấu hình `RecordingChannelAdapter` trả về `Err(ChannelDeliveryError::TransientNetwork)` ở lần đầu và `Ok` ở lần thứ hai $\rightarrow$ Web UI Inbox vẫn đọc được ngay lập tức từ đầu; Outbox chuyển từ `pending` $\rightarrow$ `sending` $\rightarrow$ tăng `attempts = 2` và chuyển sang `sent`.
   - Cấu hình trả về `Err(ChannelDeliveryError::Unauthorized)` $\rightarrow$ chuyển thẳng sang `failed` mà không retry vô ích.
4. **Secret Protection, `Patch<SecretValue>`, Audit & Optimistic Concurrency (`If-Match`)**:
   - `PUT /api/admin/notifications/settings` với `bot_token: "123456:ABC..."` $\rightarrow$ `GET /api/admin/notifications/settings` chỉ trả về `has_saved_token: true` và `credential_metadata.masked_key`, tuyệt đối không chứa chuỗi plaintext hay `sealed:` trong JSON trả về; đồng thời ghi nhận 1 bản ghi `Admin Audit Event`.
   - `PUT` với `bot_token: null` $\rightarrow$ xóa `encrypted_credential` về `NULL`.
   - `PUT` không truyền `bot_token` (`undefined`) $\rightarrow$ giữ nguyên `encrypted_credential` đã lưu.
   - `PUT` với `If-Match` cũ $\rightarrow$ trả về `409 Conflict` (`revision_conflict`) và ghi audit conflict.

### 8.2. Lệnh chạy kiểm thử chuẩn

```bash
# 1. Kiểm tra biên dịch và unit/HTTP tests của module notification_hub
./scripts/agent-test.sh targeted notification_hub voice-agent-server

# 2. Kiểm tra kiểu và biên dịch toàn crate voice-agent-server
./scripts/agent-test.sh check voice-agent-server

# 3. Kiểm tra Frontend (TypeScript + Vitest + Build)
./scripts/agent-test.sh web typecheck
./scripts/agent-test.sh web test
./scripts/agent-test.sh web build
```

---

## 9. Nhật ký Triển khai (Implementation Status)

### Giai đoạn 1 — Web Admin (`apps/admin-web`) với Dữ liệu Demo (`✅ ĐÃ HOÀN THÀNH`)
- **Phạm vi**:
  - Hoàn thiện toàn bộ tầng DTOs, API client (`notificationsApi`), SSE listener (`listenNotificationStream`) và **Stateful Demo Store** (`apps/admin-web/src/api/notifications.ts`).
  - Hoàn thiện giao diện chuông thông báo toàn cục (`NotificationBell.vue`) trên cả Mobile header (`AdminShell.vue`) và Desktop sidebar (`SidebarContent.vue`), cùng bảng cấu hình 3 kênh **Web UI / Telegram Bot / Zalo Bot** (`NotificationSettingsCard.vue`) tại trang `/jobs/settings` (`BackgroundPage.vue`).
  - Hoàn thiện kiểm thử đơn vị (TDD) và rà soát `/code-review` 2 trục (Standards & Spec): vượt qua `web typecheck`, `73/73` test files và `web build`.

### Giai đoạn 2 — Rust Backend (`crates/voice-agent-server`), Domain Docs & Kết nối Web (`✅ ĐÃ HOÀN THÀNH`)
- **Phạm vi**:
  - Bổ sung từ vựng chuẩn `## Multi-Channel Notifications` vào [`GLOSSARY.md`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/GLOSSARY.md) và [`CONTEXT.md`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/CONTEXT.md).
  - Triển khai migration [`0137_notification_hub.sql`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/migrations/0137_notification_hub.sql), module `services/notification_hub/` (`mod.rs`, `types.rs`, `store.rs`, `adapters.rs`, `dispatcher.rs`, `bridge.rs`, `tests.rs`) với `TelegramBotAdapter`, `ZaloBotAdapter`, `RecordingChannelAdapter`.
  - Đấu nối toàn bộ 6 chủ đề sự kiện từ `services::reminder_scheduler::start_with_hub`, `TaskManager` (`prepare_background`), `knowledge_processing::reviews` (`lint_wiki`) và `SystemMcp::propose`.
  - Mở đầy đủ 8 route Admin HTTP `/api/admin/notifications*` tại [`app/admin/notification_hub.rs`](file:///Users/lamphuchai/Desktop/project_rust/AI_Agent_Voice_rust/crates/voice-agent-server/src/app/admin/notification_hub.rs) và nối trực tiếp luồng SSE thời gian thực `listenNotificationStream` vào `NotificationBell.vue`.
