CREATE TABLE knowledge_processing_settings (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
    enabled INTEGER NOT NULL DEFAULT 0 CHECK(enabled IN (0,1)),
    analysis_provider_ref TEXT,
    publication_policy TEXT NOT NULL DEFAULT 'review_all' CHECK(publication_policy IN ('review_all','auto_publish','draft_only')),
    trigger_mode TEXT NOT NULL DEFAULT 'manual' CHECK(trigger_mode IN ('manual','auto_after_import')),
    max_candidates_per_source INTEGER NOT NULL DEFAULT 5 CHECK(max_candidates_per_source BETWEEN 1 AND 5),
    revision INTEGER NOT NULL DEFAULT 1 CHECK(revision>0)
);
INSERT INTO knowledge_processing_settings(singleton) VALUES(1);
