-- Per-Template grants: which of an Agent's enabled Templates may activate one of its Speaker
-- candidates.  The candidate row in agent_speaker_candidates is the Agent/Speaker binding; this
-- table scopes that binding to explicit Templates.  A Template unassigned later leaves the grant
-- dangling but inert, so resolution must intersect with the live assignment set.
CREATE TABLE agent_speaker_template_grants (
    agent_id INTEGER NOT NULL REFERENCES agents(id) ON DELETE CASCADE,
    speaker_id INTEGER NOT NULL REFERENCES speakers(id) ON DELETE CASCADE,
    template_id INTEGER NOT NULL REFERENCES agent_templates(id) ON DELETE CASCADE,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (agent_id, speaker_id, template_id)
);
CREATE INDEX agent_speaker_template_grants_reverse
    ON agent_speaker_template_grants(speaker_id, agent_id, template_id);
