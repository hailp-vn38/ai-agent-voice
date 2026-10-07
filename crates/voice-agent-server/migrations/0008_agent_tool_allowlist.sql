CREATE TABLE agent_speaker_policies (
 agent_id INTEGER PRIMARY KEY REFERENCES agents(id) ON DELETE CASCADE,
 mode TEXT NOT NULL DEFAULT 'off' CHECK(mode IN ('off','observe','required')),
 revision INTEGER NOT NULL DEFAULT 1
);
CREATE TABLE external_tool_observations (
 server_id INTEGER NOT NULL REFERENCES mcp_servers(id) ON DELETE CASCADE,
 original_name TEXT NOT NULL,
 description TEXT NOT NULL,
 input_schema TEXT NOT NULL,
 fingerprint TEXT NOT NULL,
 server_revision INTEGER NOT NULL,
 blocked INTEGER NOT NULL DEFAULT 0 CHECK(blocked IN (0,1)),
 revision INTEGER NOT NULL DEFAULT 1,
 observed_at INTEGER NOT NULL,
 PRIMARY KEY(server_id,original_name)
);
CREATE TABLE agent_external_tool_allowlist (
 agent_id INTEGER NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
 server_id INTEGER NOT NULL REFERENCES mcp_servers(id) ON DELETE CASCADE,
 original_name TEXT NOT NULL,
 fingerprint TEXT NOT NULL,
 allowed INTEGER NOT NULL CHECK(allowed IN (0,1)),
 sensitive INTEGER NOT NULL CHECK(sensitive IN (0,1)),
 revision INTEGER NOT NULL DEFAULT 1,
 PRIMARY KEY(agent_id,server_id,original_name)
);
