-- Memory index only. Canonical Facts are Markdown; MEMORY.md is generated.
CREATE TABLE mem_documents (
    fact_id TEXT PRIMARY KEY,
    relative_path TEXT NOT NULL UNIQUE,
    title TEXT NOT NULL,
    owner_scope TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK(revision>0),
    status TEXT NOT NULL CHECK(status IN ('active','superseded','needs_review','forgotten')),
    confirmed INTEGER NOT NULL CHECK(confirmed IN (0,1)),
    content_sha256 TEXT NOT NULL CHECK(length(content_sha256)=64),
    mtime_ns INTEGER NOT NULL,
    size_bytes INTEGER NOT NULL CHECK(size_bytes>=0),
    indexer_version INTEGER NOT NULL CHECK(indexer_version>0)
);
CREATE INDEX mem_documents_active_scope ON mem_documents(owner_scope,status,confirmed);
CREATE VIRTUAL TABLE mem_fts USING fts5(fact_id UNINDEXED,title,body,tokenize='unicode61 remove_diacritics 2');
CREATE TABLE mem_index_state (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
    health TEXT NOT NULL CHECK(health IN ('ready','stale','rebuilding','unavailable','error')),
    indexer_version INTEGER NOT NULL CHECK(indexer_version>0),
    last_complete_scan_at INTEGER,
    last_error TEXT
);
INSERT INTO mem_index_state(singleton,health,indexer_version) VALUES(1,'stale',1);
