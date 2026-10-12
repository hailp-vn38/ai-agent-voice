ALTER TABLE agents ADD COLUMN reminders_enabled INTEGER NOT NULL DEFAULT 0
  CHECK(reminders_enabled IN (0,1));

ALTER TABLE agents ADD COLUMN reminder_policy_revision INTEGER NOT NULL DEFAULT 1;

CREATE TABLE device_reminder_sync (
  device_row_id INTEGER PRIMARY KEY REFERENCES devices(id) ON DELETE CASCADE,
  schedule_revision INTEGER NOT NULL DEFAULT 0 CHECK(schedule_revision >= 0),
  last_acked_revision INTEGER NOT NULL DEFAULT 0 CHECK(last_acked_revision >= 0),
  last_acked_at_ms INTEGER,
  last_reported_wake_at_ms INTEGER
);

CREATE TABLE device_reminders (
  id TEXT PRIMARY KEY,
  device_row_id INTEGER NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
  message TEXT NOT NULL CHECK(length(trim(message)) BETWEEN 1 AND 1024),
  due_at_ms INTEGER NOT NULL,
  timezone TEXT NOT NULL DEFAULT 'UTC',
  status TEXT NOT NULL DEFAULT 'pending'
    CHECK(status IN ('pending','completed','cancelled','expired')),
  row_revision INTEGER NOT NULL DEFAULT 1 CHECK(row_revision > 0),
  schedule_revision INTEGER NOT NULL CHECK(schedule_revision > 0),
  source TEXT NOT NULL CHECK(source IN ('voice','web','web_assistant')),
  attempts_used INTEGER NOT NULL DEFAULT 0 CHECK(attempts_used >= 0),
  next_attempt_at_ms INTEGER,
  created_at_ms INTEGER NOT NULL,
  updated_at_ms INTEGER NOT NULL,
  completed_at_ms INTEGER,
  cancelled_at_ms INTEGER
);
CREATE INDEX idx_device_reminders_due
  ON device_reminders(status, due_at_ms, next_attempt_at_ms, device_row_id);
CREATE INDEX idx_device_reminders_device
  ON device_reminders(device_row_id, status, due_at_ms);

CREATE TABLE reminder_delivery_attempts (
  attempt_id TEXT PRIMARY KEY,
  reminder_id TEXT NOT NULL REFERENCES device_reminders(id) ON DELETE CASCADE,
  session_instance_id TEXT NOT NULL,
  status TEXT NOT NULL CHECK(status IN ('claimed','played','failed','uncertain')),
  lease_until_ms INTEGER NOT NULL,
  error_code TEXT,
  created_at_ms INTEGER NOT NULL,
  updated_at_ms INTEGER NOT NULL
);
CREATE INDEX idx_reminder_attempt_lease
  ON reminder_delivery_attempts(status, lease_until_ms);

-- Idempotency bền vững: request retry sau crash không tạo lịch thứ hai.
CREATE TABLE reminder_mutation_keys (
  caller_namespace TEXT NOT NULL,
  request_id TEXT NOT NULL,
  operation TEXT NOT NULL,
  device_row_id INTEGER NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
  payload_hash TEXT NOT NULL,
  response_json TEXT NOT NULL,
  created_at_ms INTEGER NOT NULL,
  PRIMARY KEY (caller_namespace, request_id, operation, device_row_id)
);

-- Chỉ những ID đã được firmware NVS ACK; snapshot >64 là out_of_window.
CREATE TABLE device_reminder_cached_items (
  device_row_id INTEGER NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
  reminder_id TEXT NOT NULL REFERENCES device_reminders(id) ON DELETE CASCADE,
  snapshot_revision INTEGER NOT NULL,
  PRIMARY KEY(device_row_id, reminder_id)
);

CREATE INDEX idx_reminder_mutation_retention ON reminder_mutation_keys(created_at_ms);
