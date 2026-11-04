CREATE TABLE device_enrollments (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    device_id TEXT NOT NULL,
    client_id TEXT NOT NULL,
    code TEXT NOT NULL UNIQUE CHECK (length(code) = 6 AND code NOT GLOB '*[^0-9]*'),
    challenge TEXT NOT NULL,
    metadata_json TEXT NOT NULL CHECK (json_valid(metadata_json) AND length(CAST(metadata_json AS BLOB)) <= 16384),
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'claimed', 'expired', 'cancelled')),
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL CHECK (expires_at > created_at),
    terminal_at INTEGER,
    claimed_device_id INTEGER REFERENCES devices(id) ON DELETE SET NULL,
    CHECK ((status = 'pending' AND terminal_at IS NULL AND claimed_device_id IS NULL) OR (status <> 'pending' AND terminal_at IS NOT NULL)),
    CHECK (status = 'claimed' OR claimed_device_id IS NULL)
);
CREATE UNIQUE INDEX device_enrollments_one_pending_device ON device_enrollments(device_id) WHERE status = 'pending';
CREATE INDEX device_enrollments_pending_expiry ON device_enrollments(expires_at, id) WHERE status = 'pending';
CREATE INDEX device_enrollments_terminal_retention ON device_enrollments(terminal_at, id) WHERE status <> 'pending';
