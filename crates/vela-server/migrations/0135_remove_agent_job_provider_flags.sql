-- Agent Job execution moved to orchestrator-agent; remove legacy provider capability flags.
UPDATE providers
SET config_json = json_remove(config_json, '$.supports_responses', '$.supports_structured_outputs'),
    revision = revision + 1,
    updated_at = unixepoch()
WHERE adapter IN ('openai', 'opencode_free')
  AND (
    json_type(config_json, '$.supports_responses') IS NOT NULL
    OR json_type(config_json, '$.supports_structured_outputs') IS NOT NULL
  );
