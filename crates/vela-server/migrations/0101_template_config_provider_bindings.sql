-- Preserve existing bindings and the AUTOINCREMENT high-water mark.
CREATE TEMP TABLE config_binding_sequence AS SELECT seq FROM sqlite_sequence WHERE name='template_provider_bindings';
CREATE TEMP TABLE saved_config_bindings AS SELECT * FROM template_provider_bindings;
DROP TABLE template_provider_bindings;
CREATE TABLE template_provider_bindings (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    template_id INTEGER NOT NULL REFERENCES agent_templates(id) ON DELETE CASCADE,
    provider_type TEXT NOT NULL CHECK (provider_type IN ('vad','asr','llm','tts','speaker')),
    provider_source TEXT NOT NULL DEFAULT 'database' CHECK (provider_source IN ('database','config')),
    provider_id INTEGER REFERENCES providers(id) ON DELETE RESTRICT,
    config_provider_key TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE(template_id,provider_type),
    CHECK ((provider_source='database' AND provider_id IS NOT NULL AND config_provider_key IS NULL)
        OR (provider_source='config' AND provider_id IS NULL AND config_provider_key IS NOT NULL AND length(config_provider_key)>0))
);
INSERT INTO template_provider_bindings (id,template_id,provider_type,provider_id,created_at,updated_at)
SELECT id,template_id,provider_type,provider_id,created_at,updated_at FROM saved_config_bindings;
UPDATE sqlite_sequence SET seq=max(seq,coalesce((SELECT seq FROM config_binding_sequence),0)) WHERE name='template_provider_bindings';
INSERT INTO sqlite_sequence(name,seq) SELECT 'template_provider_bindings',seq FROM config_binding_sequence WHERE NOT EXISTS(SELECT 1 FROM sqlite_sequence WHERE name='template_provider_bindings');
DROP TABLE saved_config_bindings;
DROP TABLE config_binding_sequence;
CREATE TEMP TABLE config_binding_fk_check(violations INTEGER CHECK (violations=0));
INSERT INTO config_binding_fk_check SELECT count(*) FROM pragma_foreign_key_check;
DROP TABLE config_binding_fk_check;
