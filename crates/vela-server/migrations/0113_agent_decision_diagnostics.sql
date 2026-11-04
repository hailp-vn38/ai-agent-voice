-- Extend metadata-only Agent Job events without persisting raw LLM output.
CREATE TABLE job_events_decision_v2 (
    job_id TEXT NOT NULL REFERENCES jobs(id),
    sequence INTEGER NOT NULL CHECK(sequence > 0),
    event_type TEXT NOT NULL CHECK(event_type IN (
      'plan_attempt','plan_rejected','plan_accepted','step_started','step_finished','job_finished',
      'llm_run_started','llm_provider_acquired','llm_request_started','llm_response_received','llm_run_finished',
      'agent_run_created','agent_run_finished','join_completed',
      'agent_decision_accepted','agent_decision_rejected'
    )),
    created_at INTEGER NOT NULL,
    step_key TEXT,
    payload_json TEXT NOT NULL CHECK(json_valid(payload_json) AND length(payload_json) <= 4096),
    PRIMARY KEY(job_id,sequence)
);
INSERT INTO job_events_decision_v2(job_id,sequence,event_type,created_at,step_key,payload_json)
SELECT job_id,sequence,event_type,created_at,step_key,payload_json FROM job_events;
DROP TABLE job_events;
ALTER TABLE job_events_decision_v2 RENAME TO job_events;
CREATE UNIQUE INDEX job_join_once ON job_events(job_id,json_extract(payload_json,'$.fork_group_id'))
  WHERE event_type='join_completed';
