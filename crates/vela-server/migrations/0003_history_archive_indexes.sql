CREATE INDEX idx_history_messages_device_time
ON history_messages(device_id, created_at DESC);

CREATE INDEX idx_history_messages_agent_time
ON history_messages(agent_id, created_at DESC);

CREATE INDEX idx_history_messages_template_time
ON history_messages(template_id, created_at DESC);

CREATE INDEX idx_history_messages_session_turn
ON history_messages(session_id, turn_id);
