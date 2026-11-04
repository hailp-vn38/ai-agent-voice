CREATE TABLE speaker_quick_captures (
    id TEXT PRIMARY KEY,
    provider_id INTEGER REFERENCES providers(id) ON DELETE SET NULL,
    provider_revision INTEGER NOT NULL CHECK (provider_revision > 0),
    runtime_id TEXT NOT NULL,
    embedding_space TEXT NOT NULL,
    dims INTEGER NOT NULL CHECK (dims > 0),
    vector BLOB,
    status TEXT NOT NULL CHECK (status IN ('accepted', 'committed')),
    speaker_id INTEGER REFERENCES speakers(id) ON DELETE SET NULL,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL CHECK (expires_at > created_at),
    committed_at INTEGER,
    CHECK ((status = 'accepted' AND vector IS NOT NULL AND speaker_id IS NULL AND committed_at IS NULL)
        OR (status = 'committed' AND vector IS NULL AND speaker_id IS NOT NULL AND committed_at IS NOT NULL))
);

CREATE INDEX idx_speaker_quick_captures_expiry
    ON speaker_quick_captures(status, expires_at);
