-- Operational proposal metadata only. Canonical bodies stay in Wiki/Drafts Markdown.
-- Job IDs are retained after explicit Job-history deletion for the decision audit.
CREATE TABLE knowledge_proposals (
    id TEXT PRIMARY KEY,
    job_id TEXT NOT NULL,
    candidate_index INTEGER NOT NULL CHECK(candidate_index BETWEEN 0 AND 4),
    source_ref TEXT NOT NULL,
    source_sha256 TEXT NOT NULL CHECK(length(source_sha256)=64),
    wiki_id TEXT NOT NULL,
    content_sha256 TEXT NOT NULL CHECK(length(content_sha256)=64),
    size_bytes INTEGER NOT NULL CHECK(size_bytes BETWEEN 1 AND 16384),
    draft_revision INTEGER,
    canonical_base_revision INTEGER,
    policy_revision INTEGER NOT NULL,
    publication_policy TEXT NOT NULL CHECK(publication_policy IN ('review_all','auto_publish','draft_only')),
    state TEXT NOT NULL CHECK(state IN ('staging','pending_review','publishing','published','rejected','needs_reconcile')),
    reasons_json TEXT NOT NULL CHECK(json_valid(reasons_json) AND length(reasons_json)<=2048),
    evidence_json TEXT NOT NULL CHECK(json_valid(evidence_json) AND length(evidence_json)<=32768),
    revision INTEGER NOT NULL DEFAULT 1 CHECK(revision>0),
    actor TEXT,
    decided_at INTEGER,
    accepted_revision INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE(job_id,candidate_index)
);
CREATE UNIQUE INDEX knowledge_proposal_target ON knowledge_proposals(wiki_id)
    WHERE state IN ('staging','pending_review','publishing','needs_reconcile');
CREATE INDEX knowledge_proposal_queue ON knowledge_proposals(state,id);
CREATE TRIGGER knowledge_proposal_final BEFORE UPDATE ON knowledge_proposals
WHEN OLD.state IN ('published','rejected')
BEGIN SELECT RAISE(ABORT,'proposal_terminal'); END;
