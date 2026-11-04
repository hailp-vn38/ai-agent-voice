-- Admin resolution is durable metadata; canonical draft/history bytes stay in the vault.
CREATE TABLE knowledge_draft_resolutions (
    id TEXT PRIMARY KEY,
    actor TEXT NOT NULL,
    request_id TEXT NOT NULL,
    operation_key TEXT NOT NULL,
    request_fingerprint TEXT NOT NULL,
    wiki_id TEXT NOT NULL,
    owner_job_id TEXT NOT NULL,
    action TEXT NOT NULL CHECK(action IN ('adopt','discard')),
    expected_draft_revision INTEGER NOT NULL CHECK(expected_draft_revision>0),
    expected_base_revision INTEGER,
    expected_content_sha256 TEXT NOT NULL,
    record_sha256 TEXT NOT NULL,
    relative_path TEXT NOT NULL,
    staged INTEGER NOT NULL CHECK(staged IN (0,1)),
    phase TEXT NOT NULL CHECK(phase IN ('reserved','committed','unknown')),
    outcome_json TEXT CHECK(outcome_json IS NULL OR (json_valid(outcome_json) AND length(CAST(outcome_json AS BLOB))<=16384)),
    created_at INTEGER NOT NULL,
    UNIQUE(actor,operation_key)
);
CREATE INDEX knowledge_resolution_owner ON knowledge_draft_resolutions(owner_job_id,wiki_id,phase);
CREATE TRIGGER knowledge_resolution_contract_immutable BEFORE UPDATE ON knowledge_draft_resolutions
WHEN NEW.id IS NOT OLD.id OR NEW.actor IS NOT OLD.actor OR NEW.request_id IS NOT OLD.request_id
 OR NEW.operation_key IS NOT OLD.operation_key OR NEW.request_fingerprint IS NOT OLD.request_fingerprint
 OR NEW.wiki_id IS NOT OLD.wiki_id OR NEW.owner_job_id IS NOT OLD.owner_job_id OR NEW.action IS NOT OLD.action
 OR NEW.expected_draft_revision IS NOT OLD.expected_draft_revision OR NEW.expected_base_revision IS NOT OLD.expected_base_revision
 OR NEW.expected_content_sha256 IS NOT OLD.expected_content_sha256 OR NEW.record_sha256 IS NOT OLD.record_sha256
 OR NEW.relative_path IS NOT OLD.relative_path OR NEW.staged IS NOT OLD.staged OR NEW.created_at IS NOT OLD.created_at
 OR (OLD.phase='committed' AND (NEW.phase IS NOT OLD.phase OR NEW.outcome_json IS NOT OLD.outcome_json))
BEGIN SELECT RAISE(ABORT,'knowledge_resolution_immutable'); END;
