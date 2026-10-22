-- Knowledge index only. Wiki and immutable sources on disk are authoritative.
CREATE TABLE kb_documents (
    stable_id TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK(kind IN ('wiki','source_extract')),
    relative_path TEXT NOT NULL UNIQUE,
    title TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK(revision > 0),
    content_sha256 TEXT NOT NULL CHECK(length(content_sha256) = 64),
    mtime_ns INTEGER NOT NULL,
    size_bytes INTEGER NOT NULL CHECK(size_bytes >= 0),
    state TEXT NOT NULL,
    indexer_version INTEGER NOT NULL CHECK(indexer_version > 0)
);
CREATE INDEX kb_documents_kind_state ON kb_documents(kind,state);
CREATE VIRTUAL TABLE kb_fts USING fts5(stable_id UNINDEXED,title,body,tokenize='unicode61 remove_diacritics 2');
CREATE TABLE kb_index_state (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
    health TEXT NOT NULL CHECK(health IN ('ready','stale','rebuilding','unavailable','error')),
    indexer_version INTEGER NOT NULL CHECK(indexer_version>0),
    last_complete_scan_at INTEGER,
    last_error TEXT
);
INSERT INTO kb_index_state(singleton,health,indexer_version) VALUES(1,'stale',1);
