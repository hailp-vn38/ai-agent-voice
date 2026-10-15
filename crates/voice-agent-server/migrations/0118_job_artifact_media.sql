-- Job V3 artifacts keep their physical UUID key and use the worker output basename for display.
ALTER TABLE job_artifacts ADD COLUMN filename TEXT NOT NULL DEFAULT '';
