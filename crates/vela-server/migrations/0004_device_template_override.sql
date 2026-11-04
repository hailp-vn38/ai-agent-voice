-- A Device may select one active Template already assigned to its Agent.  NULL means use the
-- Agent default.  The application enforces assignment/enabled ownership before writing it.
ALTER TABLE devices ADD COLUMN template_id INTEGER REFERENCES agent_templates(id) ON DELETE SET NULL;
CREATE INDEX idx_devices_template_id ON devices(template_id);
