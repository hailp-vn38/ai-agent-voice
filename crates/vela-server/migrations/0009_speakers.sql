-- Speaker profiles, per-space voiceprints, Agent candidates, and ephemeral enrollment drafts.

CREATE TABLE speakers (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    key TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    description TEXT,
    enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    revision INTEGER NOT NULL DEFAULT 1 CHECK (revision > 0),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE speaker_voiceprints (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    speaker_id INTEGER NOT NULL REFERENCES speakers(id) ON DELETE CASCADE,
    embedding_space TEXT NOT NULL,
    provider_id INTEGER NOT NULL REFERENCES providers(id) ON DELETE RESTRICT,
    provider_key TEXT NOT NULL,
    provider_revision INTEGER NOT NULL CHECK (provider_revision > 0),
    dims INTEGER NOT NULL CHECK (dims > 0),
    vector BLOB NOT NULL,
    revision INTEGER NOT NULL DEFAULT 1 CHECK (revision > 0),
    sample_count INTEGER NOT NULL CHECK (sample_count > 0),
    browser_validation_status TEXT NOT NULL CHECK (browser_validation_status IN ('pending', 'passed', 'failed')),
    enrolled_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE(speaker_id, embedding_space)
);

CREATE TABLE agent_speaker_candidates (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id INTEGER NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    speaker_id INTEGER NOT NULL REFERENCES speakers(id) ON DELETE RESTRICT,
    created_at INTEGER NOT NULL,
    UNIQUE(agent_id, speaker_id)
);

-- Ephemeral web enrollment drafts. `status` stays 'collecting' until commit; committed rows are
-- retained briefly (audit trail) then swept past `terminal_at`. The startup sweep also tombstones
-- drafts whose `expires_at` has passed.
CREATE TABLE speaker_enrollment_drafts (
    id TEXT PRIMARY KEY,
    speaker_id INTEGER NOT NULL REFERENCES speakers(id) ON DELETE CASCADE,
    embedding_space TEXT NOT NULL,
    provider_id INTEGER REFERENCES providers(id) ON DELETE SET NULL,
    provider_key TEXT NOT NULL,
    provider_revision INTEGER NOT NULL CHECK (provider_revision > 0),
    loaded_provider_revision INTEGER CHECK (loaded_provider_revision IS NULL OR loaded_provider_revision > 0),
    runtime_id TEXT NOT NULL,
    sample_rate INTEGER NOT NULL CHECK (sample_rate > 0),
    dims INTEGER CHECK (dims IS NULL OR dims > 0),
    status TEXT NOT NULL DEFAULT 'collecting' CHECK (status IN ('collecting', 'committed', 'expired')),
    committed_speaker_id INTEGER REFERENCES speakers(id) ON DELETE SET NULL,
    base_speaker_revision INTEGER NOT NULL CHECK (base_speaker_revision > 0),
    base_voiceprint_revision INTEGER CHECK (base_voiceprint_revision IS NULL OR base_voiceprint_revision > 0),
    revision INTEGER NOT NULL DEFAULT 1 CHECK (revision > 0),
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL CHECK (expires_at > created_at),
    terminal_at INTEGER,
    CHECK ((status = 'collecting' AND terminal_at IS NULL) OR (status IN ('committed', 'expired') AND terminal_at IS NOT NULL))
);

CREATE TABLE speaker_enrollment_samples (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    draft_id TEXT NOT NULL REFERENCES speaker_enrollment_drafts(id) ON DELETE CASCADE,
    seq INTEGER NOT NULL CHECK (seq > 0),
    duration_ms INTEGER NOT NULL CHECK (duration_ms > 0),
    speech_ms INTEGER NOT NULL CHECK (speech_ms > 0),
    vector BLOB NOT NULL,
    created_at INTEGER NOT NULL,
    UNIQUE(draft_id, seq)
);

CREATE INDEX speaker_voiceprints_space ON speaker_voiceprints(embedding_space, speaker_id);
CREATE INDEX agent_speaker_candidates_agent ON agent_speaker_candidates(agent_id, speaker_id);
CREATE INDEX speaker_enrollment_drafts_speaker ON speaker_enrollment_drafts(speaker_id, status);
CREATE INDEX speaker_enrollment_drafts_expiry ON speaker_enrollment_drafts(expires_at, id) WHERE terminal_at IS NULL;
CREATE INDEX speaker_enrollment_drafts_terminal ON speaker_enrollment_drafts(terminal_at, id) WHERE status = 'committed';
