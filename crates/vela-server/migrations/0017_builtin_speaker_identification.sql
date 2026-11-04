-- Speaker identification is a built-in CAM++ capability, not a mutable Provider.
-- Preserve all published embeddings and Agent candidate links. Obsolete provider
-- provenance columns remain temporarily for compatibility with old reads; they
-- are nullable, not foreign keys, and new captures do not depend on providers.
-- A future cleanup may drop those columns after all callers have migrated.

CREATE TABLE speaker_voiceprints_builtin (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    speaker_id INTEGER NOT NULL REFERENCES speakers(id) ON DELETE CASCADE,
    embedding_space TEXT NOT NULL,
    provider_id INTEGER,
    provider_key TEXT NOT NULL DEFAULT 'builtin',
    provider_revision INTEGER NOT NULL DEFAULT 1,
    dims INTEGER NOT NULL CHECK (dims > 0),
    vector BLOB NOT NULL,
    revision INTEGER NOT NULL DEFAULT 1 CHECK (revision > 0),
    sample_count INTEGER NOT NULL DEFAULT 1 CHECK (sample_count > 0),
    browser_validation_status TEXT NOT NULL DEFAULT 'passed'
        CHECK (browser_validation_status IN ('pending', 'passed', 'failed')),
    enrolled_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    calibration_revision TEXT NOT NULL DEFAULT '',
    UNIQUE (speaker_id, embedding_space)
);
INSERT INTO speaker_voiceprints_builtin (
    id, speaker_id, embedding_space, provider_id, provider_key, provider_revision,
    dims, vector, revision, sample_count, browser_validation_status,
    enrolled_at, updated_at, calibration_revision
)
SELECT id, speaker_id, embedding_space, NULL, provider_key, provider_revision,
       dims, vector, revision, sample_count, 'passed',
       enrolled_at, updated_at, calibration_revision
FROM speaker_voiceprints;
DROP TABLE speaker_voiceprints;
ALTER TABLE speaker_voiceprints_builtin RENAME TO speaker_voiceprints;
CREATE INDEX speaker_voiceprints_space
    ON speaker_voiceprints(embedding_space, speaker_id);

CREATE TABLE speaker_quick_captures_builtin (
    id TEXT PRIMARY KEY,
    provider_id INTEGER,
    provider_revision INTEGER NOT NULL DEFAULT 1,
    runtime_id TEXT NOT NULL,
    embedding_space TEXT NOT NULL,
    dims INTEGER NOT NULL CHECK (dims > 0),
    vector BLOB,
    status TEXT NOT NULL CHECK (status IN ('accepted', 'committed')),
    speaker_id INTEGER REFERENCES speakers(id) ON DELETE SET NULL,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL CHECK (expires_at > created_at),
    committed_at INTEGER,
    CHECK ((status = 'accepted' AND vector IS NOT NULL
             AND speaker_id IS NULL AND committed_at IS NULL)
        OR (status = 'committed' AND vector IS NULL
             AND speaker_id IS NOT NULL AND committed_at IS NOT NULL))
);
-- Old staged embeddings are invalidated; the administrator records again.
-- Preserve consumed tombstones to keep retry behavior for existing speakers.
INSERT INTO speaker_quick_captures_builtin (
    id, provider_id, provider_revision, runtime_id, embedding_space, dims,
    vector, status, speaker_id, created_at, expires_at, committed_at
)
SELECT id, NULL, provider_revision, runtime_id, embedding_space, dims,
       NULL, status, speaker_id, created_at, expires_at, committed_at
FROM speaker_quick_captures WHERE status = 'committed';
DROP TABLE speaker_quick_captures;
ALTER TABLE speaker_quick_captures_builtin RENAME TO speaker_quick_captures;
CREATE INDEX idx_speaker_quick_captures_expiry
    ON speaker_quick_captures(status, expires_at);

-- Remove obsolete template-provider bindings before deleting their instances.
DELETE FROM template_provider_bindings WHERE provider_type = 'speaker';
DELETE FROM providers WHERE type = 'speaker';

-- A formerly Required agent must be explicitly opted into identification:
-- no voice authority survives this change.
UPDATE agent_speaker_policies
SET mode = 'off', revision = revision + 1
WHERE mode = 'required';
