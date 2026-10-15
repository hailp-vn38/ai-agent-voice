-- Deleting a Job may detach its retries without changing their execution or results.
DROP TRIGGER jobs_terminal_immutable;
CREATE TRIGGER jobs_terminal_immutable BEFORE UPDATE OF
    id,schema_version,requester_scope,device_incarnation,agent_incarnation,
    request_json,request_fingerprint,snapshot_json,plugin_choices_json,
    settings_revision,budget_limit_json,budget_used_json,execution_mode,status,
    plan_revision,deadline_at,idempotency_key,error_code,scheduler_seen_at,
    result_json,created_at,updated_at
ON jobs
WHEN OLD.status IN ('succeeded','partial_success','failed','cancelled')
BEGIN SELECT RAISE(ABORT,'job_terminal'); END;

DROP TRIGGER jobs_contract_immutable;
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
