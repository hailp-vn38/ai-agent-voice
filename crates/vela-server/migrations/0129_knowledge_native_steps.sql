ALTER TABLE job_steps ADD COLUMN tool_call_id TEXT;
DROP INDEX job_step_v2_turn;
CREATE UNIQUE INDEX job_step_legacy_turn ON job_steps(job_id,agent_turn_id) WHERE agent_turn_id IS NOT NULL AND tool_call_id IS NULL;
CREATE UNIQUE INDEX job_step_native_call ON job_steps(job_id,agent_turn_id,tool_call_id) WHERE tool_call_id IS NOT NULL;
CREATE TABLE job_step_knowledge (
    job_id TEXT NOT NULL,
    step_key TEXT NOT NULL,
    operation_key TEXT NOT NULL,
    command TEXT NOT NULL CHECK(command IN ('create','edit','publish')),
    request_fingerprint TEXT NOT NULL,
    wiki_id TEXT NOT NULL,
    expected_base_revision INTEGER,
    expected_draft_revision INTEGER,
    committed_revision INTEGER,
    content_sha256 TEXT NOT NULL,
    provenance_sha256 TEXT NOT NULL,
    phase TEXT NOT NULL CHECK(phase IN ('reserved','committed','unknown')),
    outcome_json TEXT CHECK(outcome_json IS NULL OR (json_valid(outcome_json) AND length(CAST(outcome_json AS BLOB))<=16384)),
    PRIMARY KEY(job_id,step_key),
    UNIQUE(job_id,operation_key),
    FOREIGN KEY(job_id,step_key) REFERENCES job_steps(job_id,step_key)
);
CREATE TRIGGER job_step_native_call_immutable BEFORE UPDATE OF tool_call_id ON job_steps
WHEN NEW.tool_call_id IS NOT OLD.tool_call_id
BEGIN SELECT RAISE(ABORT,'job_step_call_immutable'); END;
