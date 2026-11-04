-- Ticket 14: explicitly reloaded deployment calibration catalog and exact-candidate-set evidence.
--
-- `speaker_calibration_catalog` is the single published catalog (id = 1); no row means no
-- deployment catalog has been loaded yet, so only the built-in preliminary calibration applies.
-- Evidence is keyed by the calibration revision it was qualified under plus the exact
-- candidate-set digest, so publishing a new revision or dropping one entry only invalidates the
-- snapshots it actually covers. Voiceprint catalog generation changes do not touch this table.

CREATE TABLE speaker_calibration_catalog (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    calibration_revision TEXT NOT NULL,
    source_digest TEXT NOT NULL,
    profiles_json TEXT NOT NULL,
    published_at INTEGER NOT NULL
);

CREATE TABLE speaker_calibration_evidence (
    calibration_revision TEXT NOT NULL,
    candidate_set_digest TEXT NOT NULL,
    candidate_set_json TEXT NOT NULL,
    report_ref TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (calibration_revision, candidate_set_digest)
);
