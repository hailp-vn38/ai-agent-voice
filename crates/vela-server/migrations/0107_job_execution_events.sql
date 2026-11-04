-- Durable, admin-only metadata timeline. No raw model output, prompt, or plugin stdout.
CREATE TABLE job_events (
    job_id TEXT NOT NULL REFERENCES jobs(id),
    sequence INTEGER NOT NULL CHECK(sequence > 0),
    event_type TEXT NOT NULL CHECK(event_type IN (
        'plan_attempt','plan_rejected','plan_accepted',
        'step_started','step_finished','job_finished'
    )),
    created_at INTEGER NOT NULL,
    step_key TEXT,
    payload_json TEXT NOT NULL CHECK(json_valid(payload_json) AND length(payload_json) <= 4096),
    PRIMARY KEY(job_id,sequence)
);
