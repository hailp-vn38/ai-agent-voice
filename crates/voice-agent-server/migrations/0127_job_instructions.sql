-- Immutable input resources, separate from the bounded Job snapshot and Wiki.
CREATE TABLE job_instruction_snapshots (
    job_id TEXT NOT NULL REFERENCES jobs(id),
    profile TEXT NOT NULL CHECK(profile IN ('root','child','knowledge')),
    identity TEXT NOT NULL CHECK(length(identity)=64),
    payload_json TEXT NOT NULL CHECK(json_valid(payload_json) AND length(CAST(payload_json AS BLOB))<=262144),
    PRIMARY KEY(job_id,profile)
);
CREATE TRIGGER job_instruction_snapshot_immutable BEFORE UPDATE ON job_instruction_snapshots
BEGIN SELECT RAISE(ABORT,'job_instruction_immutable'); END;
