# ADR 0118 — Canonical Markdown and immutable source revisions

Status: Accepted. Supersedes the former SQLite-first Wiki proposal.

Wiki Pages, Memory Facts and versioned original Sources are canonical files, not rows in a central Wiki database. Rust Core validates stable IDs, expected revisions, scoped ownership and provenance, then atomically publishes Markdown with immutable prior revision snapshots. SQLite full-text content is a **rebuildable projection** only. Search must report degraded health when reconciliation fails; direct reads use canonical files. Neither a Job Artifact nor a short content hash is stable Wiki identity.

A canonical filesystem rename and a SQLite transaction cannot be atomically committed together, so recovery reconciles files and indexes; optional Git does not replace backups of original binaries.

## Implemented first source format

Only UTF-8 Markdown Source v1 is initially imported. The raw Source original, extracted text copy and SHA256 manifest live under `raw/src_UUID/1`. A Source must be successfully verified before it can back a Wiki publication. Formats requiring extraction (PDF/DOCX/images/audio/video) are future adapters, not a reason to weaken original/provenance immutability.
