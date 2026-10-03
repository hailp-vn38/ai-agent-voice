CREATE TABLE agents (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    key TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    description TEXT,
    enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    revision INTEGER NOT NULL DEFAULT 1 CHECK (revision > 0),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE agent_templates (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    key TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    description TEXT,
    language TEXT NOT NULL,
    prompt TEXT NOT NULL,
    enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    revision INTEGER NOT NULL DEFAULT 1 CHECK (revision > 0),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE agent_template_assignments (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id INTEGER NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    template_id INTEGER NOT NULL REFERENCES agent_templates(id) ON DELETE CASCADE,
    is_default INTEGER NOT NULL DEFAULT 0 CHECK (is_default IN (0, 1)),
    enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    created_at INTEGER NOT NULL,
    UNIQUE(agent_id, template_id)
);

CREATE TABLE providers (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    key TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    type TEXT NOT NULL CHECK (type IN ('vad', 'asr', 'llm', 'tts')),
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
    provider_type TEXT NOT NULL CHECK (provider_type IN ('vad', 'asr', 'llm', 'tts')),
    provider_id INTEGER NOT NULL REFERENCES providers(id) ON DELETE RESTRICT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE(template_id, provider_type)
);

CREATE TABLE mcp_servers (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    key TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL,
    transport TEXT NOT NULL DEFAULT 'streamable_http' CHECK (transport = 'streamable_http'),
    url TEXT NOT NULL,
    headers_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(headers_json)),
    auth_type TEXT NOT NULL DEFAULT 'none' CHECK (auth_type IN ('none', 'bearer', 'header')),
    auth_header_name TEXT,
    secret_ref TEXT,
    connect_timeout_ms INTEGER NOT NULL DEFAULT 5000 CHECK (connect_timeout_ms > 0),
    request_timeout_ms INTEGER NOT NULL DEFAULT 30000 CHECK (request_timeout_ms > 0),
    enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    revision INTEGER NOT NULL DEFAULT 1 CHECK (revision > 0),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE agent_mcp_bindings (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id INTEGER NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    mcp_server_id INTEGER NOT NULL REFERENCES mcp_servers(id) ON DELETE CASCADE,
    enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    required INTEGER NOT NULL DEFAULT 0 CHECK (required IN (0, 1)),
    created_at INTEGER NOT NULL,
    UNIQUE(agent_id, mcp_server_id)
);

CREATE TABLE devices (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    device_id TEXT NOT NULL UNIQUE,
    agent_id INTEGER NOT NULL REFERENCES agents(id) ON DELETE RESTRICT,
    name TEXT,
    description TEXT,
    enabled INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    metadata_json TEXT CHECK (metadata_json IS NULL OR json_valid(metadata_json)),
    revision INTEGER NOT NULL DEFAULT 1 CHECK (revision > 0),
    last_seen_at INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE history_messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL,
    device_id INTEGER NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    agent_id INTEGER NOT NULL REFERENCES agents(id) ON DELETE RESTRICT,
    template_id INTEGER REFERENCES agent_templates(id) ON DELETE SET NULL,
    sequence INTEGER NOT NULL CHECK (sequence > 0),
    turn_id TEXT,
    role TEXT NOT NULL CHECK (role IN ('user', 'assistant')),
    text TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    UNIQUE(session_id, sequence)
);

CREATE TABLE admin_audit_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    created_at INTEGER NOT NULL,
    request_id TEXT NOT NULL,
    resource_type TEXT NOT NULL,
    resource_id INTEGER,
    action TEXT NOT NULL,
    prior_revision INTEGER,
    new_revision INTEGER,
    outcome TEXT NOT NULL,
    error_kind TEXT,
    affected_rows INTEGER
);
