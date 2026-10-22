-- Derived Markdown passages for verified canonical Wiki pages only.
-- The canonical Markdown and its stable/revision identity remain authoritative.
CREATE TABLE kb_chunks (
    chunk_id INTEGER PRIMARY KEY,
    stable_id TEXT NOT NULL REFERENCES kb_documents(stable_id) ON DELETE CASCADE,
    revision INTEGER NOT NULL CHECK(revision > 0),
    ordinal INTEGER NOT NULL CHECK(ordinal >= 0),
    heading_path TEXT NOT NULL,
    start_byte INTEGER NOT NULL CHECK(start_byte >= 0),
    end_byte INTEGER NOT NULL CHECK(end_byte > start_byte),
    start_line INTEGER NOT NULL CHECK(start_line > 0),
    end_line INTEGER NOT NULL CHECK(end_line >= start_line),
    chunk_sha256 TEXT NOT NULL CHECK(length(chunk_sha256) = 64),
    body TEXT NOT NULL,
    UNIQUE(stable_id, ordinal)
);
CREATE INDEX kb_chunks_page_revision ON kb_chunks(stable_id, revision, ordinal);
CREATE VIRTUAL TABLE kb_chunks_fts USING fts5(
    chunk_id UNINDEXED,
    title,
    heading_path,
    body,
    tokenize='unicode61 remove_diacritics 2'
);
