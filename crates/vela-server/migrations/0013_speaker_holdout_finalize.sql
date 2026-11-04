-- Ticket 07: holdout validation state and published speaker catalog revision.
--
-- Validation is pinned to the draft revision it was computed for, so any sample mutation
-- (revision bump) implicitly expires it. Only the decision and provenance are stored, never a
-- second copy of the holdout or centroid vector.

ALTER TABLE speaker_enrollment_samples ADD COLUMN pcm_digest TEXT NOT NULL DEFAULT '';

ALTER TABLE speaker_voiceprints ADD COLUMN calibration_revision TEXT NOT NULL DEFAULT '';

ALTER TABLE speaker_enrollment_drafts ADD COLUMN validation_status TEXT NOT NULL DEFAULT 'none';
ALTER TABLE speaker_enrollment_drafts ADD COLUMN validation_revision INTEGER NOT NULL DEFAULT 0;
ALTER TABLE speaker_enrollment_drafts ADD COLUMN validation_calibration_revision TEXT;
ALTER TABLE speaker_enrollment_drafts ADD COLUMN validation_runtime_id TEXT;
ALTER TABLE speaker_enrollment_drafts ADD COLUMN validation_provider_revision INTEGER;
ALTER TABLE speaker_enrollment_drafts ADD COLUMN holdout_digest TEXT;

-- Single-row monotonic generation bumped inside the finalize transaction. A rollback leaves it
-- unchanged, so readers never combine a new generation with old state.
CREATE TABLE speaker_catalog (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    revision INTEGER NOT NULL
);
INSERT INTO speaker_catalog (id, revision) VALUES (1, 0);
