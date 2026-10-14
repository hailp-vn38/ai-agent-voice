-- Extend metadata-only per-Job timeline; keep legacy sequences and constraints.
-- Forward SQLx migration, preserving all old Job Event rows.
CREATE TABLE job_events_llm_activity (
    job_id TEXT NOT NULL REFERENCES jobs(id),
    sequence INTEGER NOT NULL CHECK(sequence > 0),
    event_type TEXT NOT NULL CHECK(event_type IN (
        'plan_attempt','plan_rejected','plan_accepted',
        'step_started','step_finished','job_finished',
        'llm_run_started','llm_provider_acquired','llm_request_started',
        'llm_response_received','llm_run_finished'
    )),
    created_at INTEGER NOT NULL,
    step_key TEXT,
    payload_json TEXT NOT NULL CHECK(json_valid(payload_json) AND length(payload_json) <= 4096),
    PRIMARY KEY(job_id,sequence)
);
INSERT INTO job_events_llm_activity(job_id,sequence,event_type,created_at,step_key,payload_json)
SELECT job_id,sequence,event_type,created_at,step_key,payload_json FROM job_events;
DROP TABLE job_events;
ALTER TABLE job_events_llm_activity RENAME TO job_events;
