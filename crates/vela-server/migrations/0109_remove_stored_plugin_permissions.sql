-- Removed manifest fields must not prevent listing existing installations.
UPDATE plugin_installations
SET operations_json = json_remove(operations_json, '$.permissions')
WHERE json_type(operations_json, '$.permissions') IS NOT NULL;
