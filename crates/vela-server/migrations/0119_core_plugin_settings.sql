-- Built-in Core groups are compiled into the Rust server. Only mutable
-- administrator enablement is persisted here, never installation identities.
CREATE TABLE core_plugin_settings (
    plugin_id TEXT PRIMARY KEY,
    enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
    revision INTEGER NOT NULL CHECK (revision > 0),
    updated_at INTEGER NOT NULL
);

INSERT INTO core_plugin_settings(plugin_id, enabled, revision, updated_at)
VALUES
    ('core.web', 1, 1, 0),
    ('core.document', 1, 1, 0),
    ('core.data', 1, 1, 0),
    ('core.artifact', 1, 1, 0);
