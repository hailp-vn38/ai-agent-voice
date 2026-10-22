ALTER TABLE job_step_knowledge ADD COLUMN relative_path TEXT NOT NULL DEFAULT '' CHECK(length(CAST(relative_path AS BLOB))<=512);
