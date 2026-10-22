-- Rebuildable Graph View projection; canonical links live in wiki/*.md Markdown.
CREATE TABLE kb_links (
    source_id TEXT NOT NULL REFERENCES kb_documents(stable_id) ON DELETE CASCADE,
    ordinal INTEGER NOT NULL CHECK(ordinal >= 0),
    source_line INTEGER NOT NULL CHECK(source_line > 0),
    raw_target TEXT NOT NULL,
    target_id TEXT REFERENCES kb_documents(stable_id) ON DELETE CASCADE,
    link_kind TEXT NOT NULL CHECK(link_kind IN ('wikilink', 'embed', 'markdown')),
    resolution TEXT NOT NULL CHECK(resolution IN ('resolved', 'missing', 'ambiguous', 'invalid')),
    PRIMARY KEY (source_id, ordinal),
    CHECK ((resolution = 'resolved') = (target_id IS NOT NULL))
);
CREATE INDEX kb_links_target ON kb_links(target_id) WHERE target_id IS NOT NULL;
