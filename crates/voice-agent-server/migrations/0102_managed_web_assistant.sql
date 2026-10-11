ALTER TABLE devices ADD COLUMN kind TEXT NOT NULL DEFAULT 'physical'
    CHECK (kind IN ('physical', 'web_assistant'));
CREATE UNIQUE INDEX ux_devices_single_web_assistant ON devices(kind) WHERE kind='web_assistant';
