-- Value settings are independent of Core activation and External installation state.
-- Immutable versions remain reachable while pinned by Jobs.
CREATE TABLE plugin_configs (
    plugin_key TEXT PRIMARY KEY,
    current_revision INTEGER NOT NULL CHECK(current_revision > 0),
    updated_at INTEGER NOT NULL
);
CREATE TABLE plugin_config_versions (
    plugin_key TEXT NOT NULL REFERENCES plugin_configs(plugin_key),
    revision INTEGER NOT NULL CHECK(revision > 0),
    schema_digest TEXT NOT NULL,
    package_digest TEXT,
    public_json TEXT NOT NULL CHECK(json_valid(public_json) AND length(public_json) <= 16384),
    -- Each map value is an AES-GCM sealed reference, never plaintext.
    secrets_json TEXT NOT NULL CHECK(json_valid(secrets_json) AND length(secrets_json) <= 32768),
    created_at INTEGER NOT NULL,
    PRIMARY KEY(plugin_key,revision)
);
CREATE TABLE job_plugin_config_pins (
    job_id TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    plugin_key TEXT NOT NULL,
    revision INTEGER NOT NULL,
    PRIMARY KEY(job_id,plugin_key),
    FOREIGN KEY(plugin_key,revision)
        REFERENCES plugin_config_versions(plugin_key,revision)
);
CREATE INDEX job_plugin_config_pins_config ON job_plugin_config_pins(plugin_key,revision);
