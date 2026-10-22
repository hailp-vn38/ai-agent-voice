-- Session identity/provenance and transcript content are immutable. Deletion
-- remains available to the explicit terminal Job retention/deletion path.
CREATE TRIGGER job_agent_session_contract_immutable BEFORE UPDATE ON job_agent_sessions
WHEN NEW.session_id IS NOT OLD.session_id OR NEW.job_id IS NOT OLD.job_id
 OR NEW.parent_session_id IS NOT OLD.parent_session_id OR NEW.agent_key IS NOT OLD.agent_key
 OR NEW.provider_route_json IS NOT OLD.provider_route_json OR NEW.created_at IS NOT OLD.created_at
BEGIN SELECT RAISE(ABORT,'agent_session_contract_immutable'); END;
CREATE TRIGGER job_agent_message_immutable BEFORE UPDATE ON job_agent_messages
BEGIN SELECT RAISE(ABORT,'agent_message_immutable'); END;
CREATE UNIQUE INDEX job_agent_session_single_runner ON job_agent_runs(session_id)
WHERE session_id IS NOT NULL AND status IN ('queued','running');
CREATE TRIGGER job_agent_run_session_owner BEFORE UPDATE OF session_id ON job_agent_runs
WHEN NEW.session_id IS NOT NULL AND NOT EXISTS (
 SELECT 1 FROM job_agent_sessions s WHERE s.job_id=NEW.job_id AND s.session_id=NEW.session_id
 AND s.agent_key=NEW.role AND (
  (NEW.parent_agent_run_id IS NULL AND s.parent_session_id IS NULL) OR
  EXISTS (SELECT 1 FROM job_agent_runs p WHERE p.job_id=NEW.job_id
    AND p.agent_run_id=NEW.parent_agent_run_id AND p.session_id=s.parent_session_id)
 ))
BEGIN SELECT RAISE(ABORT,'agent_session_owner'); END;
CREATE TRIGGER job_agent_run_session_insert_owner BEFORE INSERT ON job_agent_runs
WHEN NEW.session_id IS NOT NULL AND NOT EXISTS (
 SELECT 1 FROM job_agent_sessions s WHERE s.job_id=NEW.job_id AND s.session_id=NEW.session_id
 AND s.agent_key=NEW.role AND (
  (NEW.parent_agent_run_id IS NULL AND s.parent_session_id IS NULL) OR
  EXISTS (SELECT 1 FROM job_agent_runs p WHERE p.job_id=NEW.job_id
    AND p.agent_run_id=NEW.parent_agent_run_id AND p.session_id=s.parent_session_id)
 ))
BEGIN SELECT RAISE(ABORT,'agent_session_owner'); END;
-- Startup migrations use a dedicated FK-disabled connection for table rebuilds.
-- Validate all retained references before committing this upgrade.
CREATE TEMP TABLE v3_fk_guard (violations INTEGER CHECK(violations=0));
INSERT INTO v3_fk_guard SELECT count(*) FROM pragma_foreign_key_check;
DROP TABLE v3_fk_guard;
