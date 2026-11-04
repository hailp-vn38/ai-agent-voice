-- Durable Job ownership is independent of Device/Agent lifetime. Their AUTOINCREMENT
-- row IDs are non-reusable incarnations; historical Jobs deliberately have no identity FK.
CREATE TABLE background_settings (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    enabled INTEGER NOT NULL DEFAULT 0 CHECK (enabled IN (0,1)),
    orchestrator_llm_provider TEXT,
    worker_llm_provider TEXT,
    revision INTEGER NOT NULL DEFAULT 1 CHECK (revision > 0)
);
INSERT INTO background_settings(singleton) VALUES(1);

CREATE TABLE plugin_installations (
    plugin_id TEXT PRIMARY KEY CHECK(length(plugin_id) BETWEEN 1 AND 128),
    version TEXT NOT NULL,
    package_digest TEXT NOT NULL,
    image_digest TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('disabled','ready','error')),
    revision INTEGER NOT NULL CHECK(revision > 0),
    approved_permissions_json TEXT NOT NULL CHECK(json_valid(approved_permissions_json) AND length(approved_permissions_json) <= 65536),
    operations_json TEXT NOT NULL CHECK(json_valid(operations_json) AND length(operations_json) <= 65536),
    last_error_code TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE jobs (
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
CREATE UNIQUE INDEX jobs_idempotency ON jobs(requester_scope,idempotency_key) WHERE idempotency_key IS NOT NULL;
CREATE INDEX jobs_queue ON jobs(status,created_at,id);
CREATE INDEX jobs_requester ON jobs(requester_scope,created_at);

CREATE TABLE job_steps (
    job_id TEXT NOT NULL REFERENCES jobs(id),
    step_key TEXT NOT NULL,
    plan_revision INTEGER NOT NULL,
    plugin_id TEXT,
    plugin_version TEXT,
    package_digest TEXT,
    image_digest TEXT,
    operation TEXT,
    arguments_json TEXT NOT NULL DEFAULT '{}' CHECK(json_valid(arguments_json) AND length(arguments_json)<=65536),
    input_refs_json TEXT NOT NULL DEFAULT '[]' CHECK(json_valid(input_refs_json)),
    output_schema_json TEXT NOT NULL DEFAULT '{}' CHECK(json_valid(output_schema_json)),
    dependencies_json TEXT NOT NULL DEFAULT '[]' CHECK(json_valid(dependencies_json)),
    required INTEGER NOT NULL DEFAULT 1 CHECK(required IN (0,1)),
    status TEXT NOT NULL CHECK(status IN ('pending','running','succeeded','failed','skipped','cancelled')),
    attempt INTEGER NOT NULL DEFAULT 0 CHECK(attempt IN (0,1)),
    attempt_id TEXT,
    attempt_state TEXT CHECK(attempt_state IS NULL OR attempt_state IN ('reserved','start_requested','started','finished','unknown_outcome')),
    worker_instance_id TEXT,
    started_at INTEGER,
    finished_at INTEGER,
    exit_reason TEXT,
    error_code TEXT,
    result_json TEXT CHECK(result_json IS NULL OR (json_valid(result_json) AND length(result_json)<=65536)),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY(job_id,step_key)
);
CREATE INDEX job_steps_status ON job_steps(job_id,status);

CREATE TABLE job_artifacts (
    id TEXT PRIMARY KEY,
    job_id TEXT NOT NULL REFERENCES jobs(id),
    step_key TEXT,
    relative_path TEXT NOT NULL,
    media_type TEXT NOT NULL,
    size_bytes INTEGER NOT NULL CHECK(size_bytes >= 0),
    sha256 TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    retention_until INTEGER,
    FOREIGN KEY(job_id,step_key) REFERENCES job_steps(job_id,step_key)
);

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
BEGIN SELECT RAISE(ABORT,'job_contract_immutable'); END;
