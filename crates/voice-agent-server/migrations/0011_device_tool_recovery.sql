-- Ticket 13: a bounded discovery recovery batch re-observes one Device incarnation and only
-- resolves a blocked contract conflict once every required complete observation agrees.  Batches
-- carry identity, a deadline and an explicit state so stale, mixed or timed-out results can never
-- combine into a false "consistent" observation.
CREATE TABLE device_tool_recovery_batches (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    device_id    INTEGER NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    state        TEXT NOT NULL,          -- open | reviewable | failed | expired
    deadline     INTEGER NOT NULL,
    created_at   INTEGER NOT NULL,
    completed_at INTEGER
);
CREATE INDEX idx_device_tool_recovery_batches_device
    ON device_tool_recovery_batches(device_id, id);

-- One complete `tools/list` walk per observing Session.  Members are scoped by batch so observations
-- from different batches never combine, and a Session that re-walks replaces its own member instead
-- of manufacturing quorum.
CREATE TABLE device_tool_recovery_members (
    batch_id    INTEGER NOT NULL REFERENCES device_tool_recovery_batches(id) ON DELETE CASCADE,
    member_key  TEXT NOT NULL,
    tools       TEXT NOT NULL,          -- canonical JSON: original_name -> {description, input_schema, fingerprint}
    observed_at INTEGER NOT NULL,
    PRIMARY KEY (batch_id, member_key)
);

-- Evidence that a later consistent batch superseded, retained with bounded retention so a resolved
-- conflict keeps an auditable trail without growing without limit.
CREATE TABLE device_tool_observation_history (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    device_id     INTEGER NOT NULL REFERENCES devices(id) ON DELETE CASCADE,
    original_name TEXT NOT NULL,
    fingerprint   TEXT NOT NULL,
    observed_at   INTEGER NOT NULL,
    superseded_at INTEGER NOT NULL,
    batch_id      INTEGER
);
CREATE INDEX idx_device_tool_observation_history_device
    ON device_tool_observation_history(device_id, id);
