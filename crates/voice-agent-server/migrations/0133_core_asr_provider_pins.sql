-- An opt-in Core ASR group; never pretend configuration equals activation.
INSERT INTO core_plugin_settings(plugin_id,enabled,revision,updated_at)
VALUES('core.asr',0,1,0);

-- Settings are already pinned by job_plugin_config_pins; this row separately
-- freezes the exact provider incarnation/revision, not merely provider_ref.
CREATE TABLE job_asr_provider_pins (
    job_id TEXT PRIMARY KEY REFERENCES jobs(id) ON DELETE CASCADE,
    provider_ref TEXT NOT NULL CHECK(length(provider_ref) BETWEEN 1 AND 256),
    incarnation INTEGER NOT NULL,
    revision INTEGER NOT NULL CHECK(revision > 0),
    digest TEXT NOT NULL CHECK(length(digest) = 64)
);
