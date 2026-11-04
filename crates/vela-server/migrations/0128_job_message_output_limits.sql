-- Bound the final encoded bytes. Per-contract limits remain enforced by Rust;
-- the storage ceiling alone never upgrades historical Sessions.
CREATE TABLE job_agent_messages_output (
    session_id TEXT NOT NULL REFERENCES job_agent_sessions(session_id),
    sequence INTEGER NOT NULL CHECK(sequence > 0),
    kind TEXT NOT NULL CHECK(kind IN ('system','user','assistant_text','assistant_tool_calls','tool_result')),
    payload_json TEXT NOT NULL CHECK(json_valid(payload_json) AND length(CAST(payload_json AS BLOB)) <= 4194304),
    created_at INTEGER NOT NULL,
    PRIMARY KEY(session_id,sequence)
);
INSERT INTO job_agent_messages_output SELECT * FROM job_agent_messages;
DROP TABLE job_agent_messages;
ALTER TABLE job_agent_messages_output RENAME TO job_agent_messages;
CREATE TRIGGER job_agent_message_immutable BEFORE UPDATE ON job_agent_messages
BEGIN SELECT RAISE(ABORT,'agent_message_immutable'); END;
CREATE TRIGGER job_agent_message_output_limit BEFORE INSERT ON job_agent_messages
WHEN length(CAST(NEW.payload_json AS BLOB)) > COALESCE((
    SELECT MIN(json_extract(i.payload_json,'$.output_limits.message_bytes'))
    FROM job_agent_sessions s JOIN job_instruction_snapshots i ON i.job_id=s.job_id
    WHERE s.session_id=NEW.session_id
),32768)
BEGIN SELECT RAISE(ABORT,'agent_message_capacity'); END;
CREATE TEMP TABLE job_message_fk_guard (violations INTEGER CHECK(violations=0));
INSERT INTO job_message_fk_guard SELECT count(*) FROM pragma_foreign_key_check;
DROP TABLE job_message_fk_guard;
