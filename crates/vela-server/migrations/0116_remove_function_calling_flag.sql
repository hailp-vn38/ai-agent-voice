-- Native function calling no longer requires an operator capability assertion.
UPDATE providers
SET config_json = json_remove(config_json, '$.supports_function_calling'),
    revision = revision + 1,
    updated_at = unixepoch()
WHERE adapter IN ('openai', 'opencode_free')
  AND json_type(config_json, '$.supports_function_calling') IS NOT NULL;
