-- SQLite table rebuild preserves every public identity and AUTOINCREMENT high-water mark.
CREATE TEMP TABLE speaker_provider_sequence AS SELECT name,seq FROM sqlite_sequence WHERE name IN ('providers','template_provider_bindings');
CREATE TEMP TABLE saved_providers AS SELECT * FROM providers;
CREATE TEMP TABLE saved_bindings AS SELECT * FROM template_provider_bindings;
DROP TABLE template_provider_bindings;
DROP TABLE providers;
CREATE TABLE providers (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    key TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    type TEXT NOT NULL CHECK (type IN ('vad', 'asr', 'llm', 'tts', 'speaker')),
    adapter TEXT NOT NULL,
    config_json TEXT NOT NULL CHECK (json_valid(config_json)),
    secret_ref TEXT,
    enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    revision INTEGER NOT NULL DEFAULT 1 CHECK (revision > 0),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE template_provider_bindings (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    template_id INTEGER NOT NULL REFERENCES agent_templates(id) ON DELETE CASCADE,
    provider_type TEXT NOT NULL CHECK (provider_type IN ('vad', 'asr', 'llm', 'tts', 'speaker')),
    provider_id INTEGER NOT NULL REFERENCES providers(id) ON DELETE RESTRICT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE(template_id, provider_type)
);

INSERT INTO providers SELECT * FROM saved_providers;
INSERT INTO template_provider_bindings SELECT * FROM saved_bindings;
UPDATE sqlite_sequence SET seq = max(seq, coalesce((SELECT seq FROM speaker_provider_sequence WHERE name=sqlite_sequence.name),0)) WHERE name IN ('providers','template_provider_bindings');
INSERT INTO sqlite_sequence(name,seq) SELECT name,seq FROM speaker_provider_sequence WHERE name NOT IN (SELECT name FROM sqlite_sequence);
DROP TABLE saved_providers;
DROP TABLE saved_bindings;
DROP TABLE speaker_provider_sequence;
CREATE TEMP TABLE speaker_fk_check(violations INTEGER CHECK (violations=0));
INSERT INTO speaker_fk_check SELECT count(*) FROM pragma_foreign_key_check;
DROP TABLE speaker_fk_check;
