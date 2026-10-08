-- Credentials and their lookup references are deployment-owned, never stored in SQLite.
-- Clear arbitrary MCP headers, which may have held API key material.
UPDATE mcp_servers SET headers_json = '{}';
ALTER TABLE providers DROP COLUMN secret_ref;
ALTER TABLE mcp_servers DROP COLUMN secret_ref;
