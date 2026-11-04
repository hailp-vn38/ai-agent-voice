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
