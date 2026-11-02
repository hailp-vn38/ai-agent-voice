CREATE TABLE wiki_conversations (
    id TEXT PRIMARY KEY NOT NULL,
    title TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

CREATE TABLE wiki_conversation_messages (
    id TEXT PRIMARY KEY NOT NULL,
    conversation_id TEXT NOT NULL REFERENCES wiki_conversations(id) ON DELETE CASCADE,
    seq INTEGER NOT NULL,
    role TEXT NOT NULL CHECK (role IN ('user', 'assistant')),
    content TEXT NOT NULL,
    content_truncated INTEGER NOT NULL DEFAULT 0 CHECK (content_truncated IN (0, 1)),
    status TEXT NOT NULL CHECK (status IN ('running', 'done', 'stopped', 'failed', 'interrupted')),
    error_code TEXT,
    pinned_paths_json TEXT,
    tools_json TEXT,
    citations_json TEXT,
    graph_json TEXT,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    UNIQUE (conversation_id, seq)
);

CREATE INDEX idx_wiki_conversation_messages_sequence
    ON wiki_conversation_messages(conversation_id, seq);

CREATE UNIQUE INDEX idx_wiki_conversation_messages_one_running
    ON wiki_conversation_messages(conversation_id)
    WHERE status = 'running';
