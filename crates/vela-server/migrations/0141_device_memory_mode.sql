-- Per-Device Memory access level. Every existing Device stays off until an Admin enables it.
ALTER TABLE devices ADD COLUMN memory_mode TEXT NOT NULL DEFAULT 'off'
    CHECK (memory_mode IN ('off', 'read', 'read_write'));
