# ADR 0115 — Immutable Job-owned local artifacts for V3

Status: Accepted, 2026-10-15.

A successful V3 Tool Step may publish multiple immutable binary or text artifacts. Reuse the existing Job engine and its SQLite `job_artifacts` table for ownership, immutable UUID identity and Step evidence; store file bytes beneath one persistent `data/job-artifacts/<job-id>/` directory. Worker output basenames become display filenames, never storage keys. Root and Child share the Job directory and retain the existing OpenCode V3 delegation and server-derived evidence rules.

Instead of introducing a separate object store, per-media storage or generic Artifact Manager interface, reuse the existing artifact module. Streaming-copy files into sibling staging names, atomically rename within the Job directory and commit artifact metadata together with Step success. SQLite/filesystem cannot share a transaction, so interrupted or failed publications are cleaned by rollback and startup reconciliation. The existing admin-authenticated download works for all media; small files use client Blob previews. Files remain until explicit Job deletion, bounded by configured Job/global quotas. Operator backups must include both SQLite and the artifact directory.

Related: [ADR 0114](0114-opencode-agent-session-tool-delegation.md), [V3 flow](../flows/10-agent-job-opencode-v3.md).

The data directory belongs to one database. Before recovery or reconciliation, startup claims a persistent sibling `.job-artifacts-owner` marker containing the canonical database basename. A different database in the same directory is rejected, including after the original runtime stops; it must use a separate data directory. Backups preserve this marker with the database and artifact directory. Test deployments likewise use one temporary directory per database.
