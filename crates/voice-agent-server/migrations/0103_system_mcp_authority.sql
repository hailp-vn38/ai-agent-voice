CREATE TABLE system_mcp_grants (
 id INTEGER PRIMARY KEY CHECK(id=1),
 system_read INTEGER NOT NULL CHECK(system_read IN (0,1)),
 mcp_read INTEGER NOT NULL CHECK(mcp_read IN (0,1)),
 mcp_discover INTEGER NOT NULL DEFAULT 0 CHECK(mcp_discover IN (0,1)),
 mcp_manage_request INTEGER NOT NULL DEFAULT 0 CHECK(mcp_manage_request IN (0,1)),
 revision INTEGER NOT NULL DEFAULT 1 CHECK(revision>0)
);
CREATE TABLE system_mcp_token_revocations (
 jti TEXT PRIMARY KEY,
 expires_at INTEGER NOT NULL
);
