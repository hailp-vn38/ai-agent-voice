-- V3 durable Agent Sessions; legacy V2 and DAG rows retain their mode.
-- Deferred FK validation is necessary while rebuilding a referenced jobs table.
PRAGMA defer_foreign_keys = ON;
CREATE TABLE jobs_knowledge_ingest (
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
      CHECK(execution_mode IN ('dag_v1','hierarchical_v2','hierarchical_v3','plugin_operation_v3','knowledge_ingest_v3')),
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
INSERT INTO jobs_knowledge_ingest (
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
ALTER TABLE jobs_knowledge_ingest RENAME TO jobs;
CREATE UNIQUE INDEX jobs_idempotency ON jobs(requester_scope,idempotency_key) WHERE idempotency_key IS NOT NULL;
CREATE INDEX jobs_queue ON jobs(status,created_at,id);
CREATE INDEX jobs_requester ON jobs(requester_scope,created_at);
CREATE TRIGGER jobs_terminal_immutable BEFORE UPDATE OF
    id,schema_version,requester_scope,device_incarnation,agent_incarnation,
    request_json,request_fingerprint,snapshot_json,plugin_choices_json,
    settings_revision,budget_limit_json,budget_used_json,execution_mode,status,
    plan_revision,deadline_at,idempotency_key,error_code,scheduler_seen_at,
    result_json,created_at,updated_at
ON jobs
WHEN OLD.status IN ('succeeded','partial_success','failed','cancelled')
BEGIN SELECT RAISE(ABORT,'job_terminal'); END;

CREATE TRIGGER jobs_contract_immutable BEFORE UPDATE ON jobs
WHEN NEW.requester_scope IS NOT OLD.requester_scope OR NEW.device_incarnation IS NOT OLD.device_incarnation
 OR NEW.agent_incarnation IS NOT OLD.agent_incarnation OR NEW.request_json IS NOT OLD.request_json
 OR NEW.request_fingerprint IS NOT OLD.request_fingerprint OR NEW.snapshot_json IS NOT OLD.snapshot_json
 OR NEW.plugin_choices_json IS NOT OLD.plugin_choices_json OR NEW.settings_revision IS NOT OLD.settings_revision
 OR NEW.budget_limit_json IS NOT OLD.budget_limit_json OR NEW.deadline_at IS NOT OLD.deadline_at
 OR NEW.idempotency_key IS NOT OLD.idempotency_key
 OR (NEW.retry_of_job_id IS NOT OLD.retry_of_job_id AND NEW.retry_of_job_id IS NOT NULL)
 OR NEW.execution_mode IS NOT OLD.execution_mode
BEGIN SELECT RAISE(ABORT,'job_contract_immutable'); END;

-- Forward-only rebuild preserves every old row and the 0117 detachment policy.
CREATE TEMP TABLE knowledge_fk_guard(violations INTEGER CHECK(violations=0));
INSERT INTO knowledge_fk_guard SELECT count(*) FROM pragma_foreign_key_check;
DROP TABLE knowledge_fk_guard;

CREATE TABLE job_events_knowledge (
    job_id TEXT NOT NULL REFERENCES jobs(id),
    sequence INTEGER NOT NULL CHECK(sequence > 0),
    event_type TEXT NOT NULL CHECK(event_type IN (
      'plan_attempt','plan_rejected','plan_accepted','step_started','step_finished','job_finished',
      'llm_run_started','llm_provider_acquired','llm_request_started','llm_response_received','llm_run_finished',
      'agent_run_created','agent_run_finished','join_completed','agent_decision_accepted','agent_decision_rejected','knowledge_analysis_submitted','knowledge_proposal_changed'
    )),
    created_at INTEGER NOT NULL,
    step_key TEXT,
    payload_json TEXT NOT NULL CHECK(json_valid(payload_json) AND length(payload_json) <= 4096),
    PRIMARY KEY(job_id,sequence)
);
INSERT INTO job_events_knowledge(job_id,sequence,event_type,created_at,step_key,payload_json)
  SELECT job_id,sequence,event_type,created_at,step_key,payload_json FROM job_events;
DROP TABLE job_events;
ALTER TABLE job_events_knowledge RENAME TO job_events;
CREATE UNIQUE INDEX job_join_once ON job_events(job_id,json_extract(payload_json,'$.fork_group_id')) WHERE event_type='join_completed';
