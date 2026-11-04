-- Deployment settings never remain authoritative in persisted desired configuration.
-- Only known legacy fields are removed; other invalid fields still fail typed validation.
UPDATE providers
SET config_json = json_remove(config_json, '$.model', '$.num_threads'),
    revision = revision + 1,
    updated_at = unixepoch()
WHERE adapter IN ('silero_onnx', 'zipformer_sherpa', 'zerotts_onnx', 'kokoro_vi_onnx', 'gipformer_sherpa_offline')
  AND (json_type(config_json, '$.model') IS NOT NULL
    OR json_type(config_json, '$.num_threads') IS NOT NULL);

UPDATE providers
SET config_json = json_remove(config_json, '$.ws_url', '$.timeout_ms'),
    revision = revision + 1,
    updated_at = unixepoch()
WHERE adapter = 'chillaudio_ws'
  AND (json_type(config_json, '$.ws_url') IS NOT NULL
    OR json_type(config_json, '$.timeout_ms') IS NOT NULL);
