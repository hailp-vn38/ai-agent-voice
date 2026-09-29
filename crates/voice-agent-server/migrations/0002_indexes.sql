CREATE UNIQUE INDEX idx_agent_one_default_template
ON agent_template_assignments(agent_id)
WHERE is_default = 1 AND enabled = 1;

CREATE INDEX idx_devices_agent_id ON devices(agent_id);
CREATE INDEX idx_history_messages_session_id ON history_messages(session_id);
CREATE INDEX idx_history_messages_created_at ON history_messages(created_at);
CREATE INDEX idx_admin_audit_events_created_at ON admin_audit_events(created_at);
