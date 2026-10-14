-- V3 durable Agent Sessions; legacy V2 and DAG rows retain their mode.
-- Deferred FK validation is necessary while rebuilding a referenced jobs table.
PRAGMA defer_foreign_keys = ON;
CREATE TABLE jobs_v3 (
    id TEXT PRIMARY KEY,
    schema_version INTEGER NOT NULL DEFAULT 1 CHECK(schema_version = 1),
    requester_scope TEXT NOT NULL,
    device_incarnation INTEGER,
    agent_incarnation INTEGER,
    request_json TEXT NOT NULL CHECK(json_valid(request_json) AND length(request_json) <= 16384),
    request_fingerprint TEXT NOT NULL,
    snapshot_json TEXT NOT NULL CHECK(json_valid(snapshot_json) AND length(snapshot_json) <= 262144),
    plugin_choices_json TEXT NOT NULL CHECK(json_valid(plugin_choices_json) AND length(plugin_choices_json) <= 131072),
    settings_revision INTEGER NOT NULL,
    budget_limit_json TEXT NOT NULL CHECK(json_valid(budget_limit_json)),
    budget_used_json TEXT NOT NULL DEFAULT '{}' CHECK(json_valid(budget_used_json)),
    execution_mode TEXT NOT NULL DEFAULT 'dag_v1'
      CHECK(execution_mode IN ('dag_v1','hierarchical_v2','hierarchical_v3')),
    status TEXT NOT NULL CHECK(status IN ('queued','planning','running','cancelling','succeeded','partial_success','failed','cancelled')),
    plan_revision INTEGER NOT NULL DEFAULT 0,
    deadline_at INTEGER NOT NULL,
    retry_of_job_id TEXT REFERENCES jobs(id),
    idempotency_key TEXT CHECK(idempotency_key IS NULL OR length(idempotency_key) BETWEEN 1 AND 128),
    error_code TEXT,
    scheduler_seen_at INTEGER,
    result_json TEXT CHECK(result_json IS NULL OR (json_valid(result_json) AND length(result_json) <= 65536)),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    CHECK((device_incarnation IS NULL AND agent_incarnation IS NULL AND requester_scope='admin:deployment') OR
          (device_incarnation IS NOT NULL AND agent_incarnation IS NOT NULL AND requester_scope<>'admin:deployment'))
);
INSERT INTO jobs_v3 (
    id,schema_version,requester_scope,device_incarnation,agent_incarnation,request_json,
    request_fingerprint,snapshot_json,plugin_choices_json,settings_revision,budget_limit_json,
    budget_used_json,execution_mode,status,plan_revision,deadline_at,retry_of_job_id,
    idempotency_key,error_code,scheduler_seen_at,result_json,created_at,updated_at
)
SELECT id,schema_version,requester_scope,device_incarnation,agent_incarnation,request_json,
    request_fingerprint,snapshot_json,plugin_choices_json,settings_revision,budget_limit_json,
    budget_used_json,execution_mode,status,plan_revision,deadline_at,retry_of_job_id,
    idempotency_key,error_code,scheduler_seen_at,result_json,created_at,updated_at FROM jobs;
DROP TABLE jobs;
ALTER TABLE jobs_v3 RENAME TO jobs;
CREATE UNIQUE INDEX jobs_idempotency ON jobs(requester_scope,idempotency_key) WHERE idempotency_key IS NOT NULL;
CREATE INDEX jobs_queue ON jobs(status,created_at,id);
CREATE INDEX jobs_requester ON jobs(requester_scope,created_at);
CREATE TRIGGER jobs_terminal_immutable BEFORE UPDATE ON jobs
WHEN OLD.status IN ('succeeded','partial_success','failed','cancelled')
BEGIN SELECT RAISE(ABORT,'job_terminal'); END;
CREATE TRIGGER jobs_contract_immutable BEFORE UPDATE ON jobs
WHEN NEW.requester_scope IS NOT OLD.requester_scope OR NEW.device_incarnation IS NOT OLD.device_incarnation
 OR NEW.agent_incarnation IS NOT OLD.agent_incarnation OR NEW.request_json IS NOT OLD.request_json
 OR NEW.request_fingerprint IS NOT OLD.request_fingerprint OR NEW.snapshot_json IS NOT OLD.snapshot_json
 OR NEW.plugin_choices_json IS NOT OLD.plugin_choices_json OR NEW.settings_revision IS NOT OLD.settings_revision
 OR NEW.budget_limit_json IS NOT OLD.budget_limit_json OR NEW.deadline_at IS NOT OLD.deadline_at
 OR NEW.idempotency_key IS NOT OLD.idempotency_key OR NEW.retry_of_job_id IS NOT OLD.retry_of_job_id
 OR NEW.execution_mode IS NOT OLD.execution_mode
BEGIN SELECT RAISE(ABORT,'job_contract_immutable'); END;

CREATE TABLE job_agent_sessions (
    session_id TEXT PRIMARY KEY,
    job_id TEXT NOT NULL REFERENCES jobs(id),
    parent_session_id TEXT,
    agent_key TEXT NOT NULL,
    provider_route_json TEXT NOT NULL CHECK(json_valid(provider_route_json) AND length(provider_route_json) <= 4096),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE(job_id,session_id),
    FOREIGN KEY(job_id,parent_session_id) REFERENCES job_agent_sessions(job_id,session_id)
);
CREATE INDEX job_agent_session_parent ON job_agent_sessions(job_id,parent_session_id);
CREATE TABLE job_agent_messages (
    session_id TEXT NOT NULL REFERENCES job_agent_sessions(session_id),
    sequence INTEGER NOT NULL CHECK(sequence > 0),
    kind TEXT NOT NULL CHECK(kind IN ('system','user','assistant_text','assistant_tool_calls','tool_result')),
    payload_json TEXT NOT NULL CHECK(json_valid(payload_json) AND length(payload_json) <= 32768),
    created_at INTEGER NOT NULL,
    PRIMARY KEY(session_id, sequence)
);
ALTER TABLE job_agent_runs ADD COLUMN session_id TEXT REFERENCES job_agent_sessions(session_id);
CREATE INDEX job_agent_runs_session ON job_agent_runs(job_id,session_id);
DROP TRIGGER job_agent_run_contract_immutable;
CREATE TRIGGER job_agent_run_contract_immutable BEFORE UPDATE ON job_agent_runs
WHEN NEW.agent_run_id IS NOT OLD.agent_run_id OR NEW.job_id IS NOT OLD.job_id
 OR NEW.parent_agent_run_id IS NOT OLD.parent_agent_run_id OR NEW.fork_group_id IS NOT OLD.fork_group_id
 OR NEW.member_ordinal IS NOT OLD.member_ordinal OR NEW.role IS NOT OLD.role
 OR NEW.goal_excerpt IS NOT OLD.goal_excerpt OR NEW.allowed_operations_json IS NOT OLD.allowed_operations_json
 OR NEW.created_at IS NOT OLD.created_at
 OR (OLD.session_id IS NOT NULL AND NEW.session_id IS NOT OLD.session_id)
BEGIN SELECT RAISE(ABORT,'agent_run_contract_immutable'); END;
