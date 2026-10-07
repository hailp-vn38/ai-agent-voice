-- Ticket 12: observed Device tool contracts and the Agent's explicit approval of them.
--
-- Observations are evidence, not rights: they are copied only from a complete, validated MCP
-- `tools/list` walk on an admitted Device WebSocket, keyed by the Device row's AUTOINCREMENT id
-- (the incarnation).  An Agent's allowlist references that same incarnation, so deleting and
-- re-enrolling a Device can never inherit an earlier approval.
CREATE TABLE device_tool_observations (
    device_id INTEGER NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    original_name TEXT NOT NULL,
    description TEXT NOT NULL,
    input_schema TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    -- Device revision observed, so a re-enrolled/reconfigured Device makes the observation stale.
    device_revision INTEGER NOT NULL,
    -- Set when two complete discoveries disagreed (or the Device revision changed): no observation
    -- is allowed to win by arriving last, so the tool stays blocked until a later recovery batch
    -- resolves it.  Never reset by a later agreeing discovery.
    blocked INTEGER NOT NULL DEFAULT 0 CHECK (blocked IN (0, 1)),
    revision INTEGER NOT NULL DEFAULT 1,
    observed_at INTEGER NOT NULL,
    PRIMARY KEY (device_id, original_name)
);

CREATE TABLE agent_device_tool_allowlist (
    agent_id INTEGER NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    device_id INTEGER NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    original_name TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    allowed INTEGER NOT NULL DEFAULT 0 CHECK (allowed IN (0, 1)),
    sensitive INTEGER NOT NULL DEFAULT 0 CHECK (sensitive IN (0, 1)),
    revision INTEGER NOT NULL DEFAULT 1,
    PRIMARY KEY (agent_id, device_id, original_name)
);
