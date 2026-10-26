use crate::{app::admin::Patch, database::secrets::SecretValue};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

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

impl NotificationTopic {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ReminderDue => "reminder_due",
            Self::TaskCompleted => "task_completed",
            Self::TaskFailed => "task_failed",
            Self::ScheduleTriggered => "schedule_triggered",
            Self::WikiReviewRequired => "wiki_review_required",
            Self::SystemChangeRequest => "system_change_request",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "reminder_due" => Some(Self::ReminderDue),
            "task_completed" => Some(Self::TaskCompleted),
            "task_failed" => Some(Self::TaskFailed),
            "schedule_triggered" => Some(Self::ScheduleTriggered),
            "wiki_review_required" => Some(Self::WikiReviewRequired),
            "system_change_request" => Some(Self::SystemChangeRequest),
            _ => None,
        }
    }

    pub const fn label_vi(self) -> &'static str {
        match self {
            Self::ReminderDue => "Nhắc nhở",
            Self::TaskCompleted => "Tác vụ hoàn tất",
            Self::TaskFailed => "Tác vụ lỗi",
            Self::ScheduleTriggered => "Lịch định kỳ",
            Self::WikiReviewRequired => "Cần duyệt Wiki",
            Self::SystemChangeRequest => "Yêu cầu duyệt MCP",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationSeverity {
    Info,
    Success,
    Warning,
    Error,
}

impl NotificationSeverity {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Success => "success",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "info" => Some(Self::Info),
            "success" => Some(Self::Success),
            "warning" => Some(Self::Warning),
            "error" => Some(Self::Error),
            _ => None,
        }
    }

    pub const fn emoji(self) -> &'static str {
        match self {
            Self::Info => "ℹ️",
            Self::Success => "✅",
            Self::Warning => "🔔",
            Self::Error => "🚨",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationSourceKind {
    Job,
    Schedule,
    Wiki,
    System,
}

impl NotificationSourceKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Job => "job",
            Self::Schedule => "schedule",
            Self::Wiki => "wiki",
            Self::System => "system",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "job" => Some(Self::Job),
            "schedule" => Some(Self::Schedule),
            "wiki" => Some(Self::Wiki),
            "system" => Some(Self::System),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutboxDeliveryStatus {
    Pending,
    Sending,
    Sent,
    Failed,
}

impl OutboxDeliveryStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Sending => "sending",
            Self::Sent => "sent",
            Self::Failed => "failed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(Self::Pending),
            "sending" => Some(Self::Sending),
            "sent" => Some(Self::Sent),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalChannelKind {
    Telegram,
    Zalo,
}

impl ExternalChannelKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Telegram => "telegram",
            Self::Zalo => "zalo",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "telegram" => Some(Self::Telegram),
            "zalo" => Some(Self::Zalo),
            _ => None,
        }
    }

    pub const fn credential_scope(self) -> &'static str {
        match self {
            Self::Telegram => "notification_channel:telegram",
            Self::Zalo => "notification_channel:zalo",
        }
    }
}

/// Core notification content shared across input, inbox items, and outbound payloads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    #[serde(default = "default_json_object")]
    pub metadata: Value,
}

fn default_json_object() -> Value {
    Value::Object(Default::default())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    pub metadata: Value,
    pub is_read: bool,
    pub read_at_ms: Option<i64>,
    pub created_at_ms: i64,
    pub deliveries: Vec<ChannelDeliveryStatusView>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct InboxListQuery {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub unread_only: Option<bool>,
    pub topic: Option<NotificationTopic>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboxPageView {
    pub items: Vec<NotificationItemView>,
    pub total: u64,
    pub unread_count: u64,
    pub page: u32,
    pub page_size: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarkReadOutput {
    pub updated: u64,
    pub unread_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteInboxOutput {
    pub deleted: u64,
    pub unread_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebUiChannelSettingsView {
    pub enabled: bool,
    pub toast_enabled: bool,
    pub retention_days: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramChannelSettingsView {
    pub enabled: bool,
    pub chat_id: String,
    pub thread_id: Option<i64>,
    pub subscribed_topics: Vec<NotificationTopic>,
    pub has_saved_token: bool,
    pub has_env_fallback: bool,
    pub credential_metadata: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZaloChannelSettingsView {
    pub enabled: bool,
    pub recipient_id: String,
    pub subscribed_topics: Vec<NotificationTopic>,
    pub has_saved_token: bool,
    pub has_env_fallback: bool,
    pub credential_metadata: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationSettingsView {
    pub revision: i64,
    pub web_ui: WebUiChannelSettingsView,
    pub telegram: TelegramChannelSettingsView,
    pub zalo: ZaloChannelSettingsView,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebUiChannelSettingsUpdate {
    pub toast_enabled: bool,
    pub retention_days: i64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TelegramChannelSettingsUpdate {
    pub enabled: bool,
    pub chat_id: String,
    #[serde(default)]
    pub thread_id: Option<i64>,
    pub subscribed_topics: Vec<NotificationTopic>,
    #[serde(default)]
    pub bot_token: Patch<SecretValue>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ZaloChannelSettingsUpdate {
    pub enabled: bool,
    pub recipient_id: String,
    pub subscribed_topics: Vec<NotificationTopic>,
    #[serde(default)]
    pub bot_token: Patch<SecretValue>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NotificationSettingsUpdate {
    pub web_ui: WebUiChannelSettingsUpdate,
    pub telegram: TelegramChannelSettingsUpdate,
    pub zalo: ZaloChannelSettingsUpdate,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TestChannelInput {
    #[serde(default)]
    pub chat_id: Option<String>,
    #[serde(default)]
    pub thread_id: Option<i64>,
    #[serde(default)]
    pub recipient_id: Option<String>,
    #[serde(default)]
    pub bot_token: Option<SecretValue>,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestChannelOutput {
    pub ok: bool,
    pub channel: ExternalChannelKind,
    pub external_message_id: Option<String>,
    pub delivered_at_ms: i64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", content = "data", rename_all = "snake_case")]
pub enum NotificationSseEvent {
    NotificationCreated(Box<NotificationItemView>),
    UnreadCountChanged { unread_count: u64 },
}

#[derive(Debug, Error)]
pub enum NotificationHubError {
    #[error("{0}")]
    InvalidInput(&'static str),
    #[error("not_found")]
    NotFound,
    #[error("revision_conflict")]
    RevisionConflict,
    #[error("credential_encryption_unavailable")]
    EncryptionUnavailable,
    #[error("database_busy")]
    Busy,
    #[error("database_unavailable")]
    Database(#[from] sqlx::Error),
}
