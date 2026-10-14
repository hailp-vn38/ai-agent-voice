-- V2 hierarchical Agent Runs; legacy DAG jobs retain dag_v1 identity.
ALTER TABLE jobs ADD COLUMN execution_mode TEXT NOT NULL DEFAULT 'dag_v1'
  CHECK(execution_mode IN ('dag_v1','hierarchical_v2'));
DROP TRIGGER jobs_contract_immutable;
CREATE TRIGGER jobs_contract_immutable BEFORE UPDATE ON jobs
WHEN NEW.requester_scope IS NOT OLD.requester_scope OR NEW.device_incarnation IS NOT OLD.device_incarnation
 OR NEW.agent_incarnation IS NOT OLD.agent_incarnation OR NEW.request_json IS NOT OLD.request_json
 OR NEW.request_fingerprint IS NOT OLD.request_fingerprint OR NEW.snapshot_json IS NOT OLD.snapshot_json
 OR NEW.plugin_choices_json IS NOT OLD.plugin_choices_json OR NEW.settings_revision IS NOT OLD.settings_revision
 OR NEW.budget_limit_json IS NOT OLD.budget_limit_json OR NEW.deadline_at IS NOT OLD.deadline_at
 OR NEW.idempotency_key IS NOT OLD.idempotency_key OR NEW.retry_of_job_id IS NOT OLD.retry_of_job_id
 OR NEW.execution_mode IS NOT OLD.execution_mode
BEGIN SELECT RAISE(ABORT,'job_contract_immutable'); END;

CREATE TABLE job_agent_runs (
    agent_run_id TEXT PRIMARY KEY,
    job_id TEXT NOT NULL REFERENCES jobs(id),
    parent_agent_run_id TEXT,
    fork_group_id TEXT,
    member_ordinal INTEGER,
    role TEXT NOT NULL CHECK(role IN ('orchestrator','research','document','general')),
    goal_excerpt TEXT NOT NULL CHECK(length(goal_excerpt) <= 256),
    allowed_operations_json TEXT NOT NULL CHECK(json_valid(allowed_operations_json) AND length(allowed_operations_json) <= 8192),
    status TEXT NOT NULL CHECK(status IN ('queued','running','succeeded','failed','cancelled','interrupted')),
    result_json TEXT CHECK(result_json IS NULL OR (json_valid(result_json) AND length(result_json) <= 4096)),
    error_code TEXT,
    created_at INTEGER NOT NULL,
    started_at INTEGER,
    finished_at INTEGER,
    UNIQUE(job_id,agent_run_id),
    UNIQUE(job_id,fork_group_id,member_ordinal),
    FOREIGN KEY(job_id,parent_agent_run_id) REFERENCES job_agent_runs(job_id,agent_run_id),
    CHECK(
      (parent_agent_run_id IS NULL AND fork_group_id IS NULL AND member_ordinal IS NULL AND role='orchestrator')
      OR (parent_agent_run_id IS NOT NULL AND fork_group_id IS NOT NULL AND member_ordinal BETWEEN 0 AND 1 AND role<>'orchestrator')
    )
);
CREATE UNIQUE INDEX job_agent_one_root ON job_agent_runs(job_id) WHERE parent_agent_run_id IS NULL;
CREATE INDEX job_agent_children ON job_agent_runs(job_id,fork_group_id,member_ordinal);
CREATE TRIGGER job_agent_run_contract_immutable BEFORE UPDATE ON job_agent_runs
WHEN NEW.agent_run_id IS NOT OLD.agent_run_id OR NEW.job_id IS NOT OLD.job_id
 OR NEW.parent_agent_run_id IS NOT OLD.parent_agent_run_id OR NEW.fork_group_id IS NOT OLD.fork_group_id
 OR NEW.member_ordinal IS NOT OLD.member_ordinal OR NEW.role IS NOT OLD.role
 OR NEW.goal_excerpt IS NOT OLD.goal_excerpt OR NEW.allowed_operations_json IS NOT OLD.allowed_operations_json
 OR NEW.created_at IS NOT OLD.created_at
BEGIN SELECT RAISE(ABORT,'agent_run_contract_immutable'); END;

ALTER TABLE job_steps ADD COLUMN agent_run_id TEXT;
ALTER TABLE job_steps ADD COLUMN agent_turn_id TEXT;
CREATE UNIQUE INDEX job_step_v2_turn ON job_steps(job_id,agent_turn_id)
  WHERE agent_turn_id IS NOT NULL;
DROP TRIGGER job_steps_contract_immutable;
CREATE TRIGGER job_steps_contract_immutable BEFORE UPDATE ON job_steps
WHEN NEW.job_id IS NOT OLD.job_id OR NEW.step_key IS NOT OLD.step_key
 OR NEW.plan_revision IS NOT OLD.plan_revision OR NEW.plugin_id IS NOT OLD.plugin_id
 OR NEW.plugin_version IS NOT OLD.plugin_version OR NEW.package_digest IS NOT OLD.package_digest
 OR NEW.installation_digest IS NOT OLD.installation_digest OR NEW.operation IS NOT OLD.operation
 OR NEW.arguments_json IS NOT OLD.arguments_json OR NEW.input_refs_json IS NOT OLD.input_refs_json
 OR NEW.output_schema_json IS NOT OLD.output_schema_json OR NEW.dependencies_json IS NOT OLD.dependencies_json
 OR NEW.required IS NOT OLD.required OR NEW.instruction IS NOT OLD.instruction
 OR NEW.agent_run_id IS NOT OLD.agent_run_id OR NEW.agent_turn_id IS NOT OLD.agent_turn_id
BEGIN SELECT RAISE(ABORT,'job_step_contract_immutable'); END;

-- Preserve existing sequences (0111) while extending metadata-only events.
CREATE TABLE job_events_hierarchical (
    job_id TEXT NOT NULL REFERENCES jobs(id),
    sequence INTEGER NOT NULL CHECK(sequence > 0),
    event_type TEXT NOT NULL CHECK(event_type IN (
      'plan_attempt','plan_rejected','plan_accepted','step_started','step_finished','job_finished',
      'llm_run_started','llm_provider_acquired','llm_request_started','llm_response_received','llm_run_finished',
      'agent_run_created','agent_run_finished','join_completed'
    )),
    created_at INTEGER NOT NULL,
    step_key TEXT,
    payload_json TEXT NOT NULL CHECK(json_valid(payload_json) AND length(payload_json) <= 4096),
    PRIMARY KEY(job_id,sequence)
);
INSERT INTO job_events_hierarchical(job_id,sequence,event_type,created_at,step_key,payload_json)
  SELECT job_id,sequence,event_type,created_at,step_key,payload_json FROM job_events;
DROP TABLE job_events;
ALTER TABLE job_events_hierarchical RENAME TO job_events;
CREATE UNIQUE INDEX job_join_once ON job_events(job_id,json_extract(payload_json,'$.fork_group_id'))
  WHERE event_type='join_completed';
