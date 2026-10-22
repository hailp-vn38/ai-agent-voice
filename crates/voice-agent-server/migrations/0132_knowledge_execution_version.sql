-- An existing deployment keeps its saved v1 policy. This migration never
-- switches review-first work into automatic native-tool publication.
ALTER TABLE knowledge_processing_settings ADD COLUMN execution_version INTEGER NOT NULL DEFAULT 1 CHECK(execution_version IN (1,2));
