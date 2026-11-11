# ADR 0128 — Admin-Managed Job Skill Catalog with Built-in Defaults and Single-Owner Agent Grants

Status: Accepted, 2026-10-28.
Extends: [ADR 0127](0127-orchestrator-flow-task-and-wiki-runtime.md).
Amended by: [ADR 0142](0142-single-skill-discovery-and-builtin-skill-upgrades.md).

`vela-server` exposes Admin inspection and editing of Job Skills (`GET / PUT / DELETE /api/admin/background/skills/{id}`) while keeping runtime `skills/<id>/SKILL.md` files as the single source of truth shared with `orchestrator_agent_rs` (no shadow SQLite skill table) and embedding the 12 Built-in Job Skills in `crates/vela-server/builtin-skills/<id>/SKILL.md` for startup seeding and default restoration. Only Custom Job Skills (`is_builtin = false`) may be deleted; Built-in Job Skills reject `DELETE` (`builtin_skill_cannot_be_deleted`) so core Wiki and Daily workflows cannot be accidentally broken. Deleting a Custom Job Skill prunes its ID from each persisted Sub-Agent assignment that references it and writes the updated settings when needed. All mutations require HTTP `ETag` preconditions (`If-None-Match: *` for create, `If-Match` for update/delete) and are serialized under `skill_write_lock` and applied live via `Orchestrator::reload_skills()` without restarting the server or disturbing active `AgentRun` snapshots.

## Considered Options

- **SQLite shadow table for Job Skills vs. filesystem `SKILL.md` + `SkillDocument` codec**: Storing skills in SQLite and mirroring to `skills/` creates dual-source drift when files are edited or mounted on disk; keeping `skills/<id>/SKILL.md` canonical and parsing/rendering through `orchestrator_agent_rs::SkillDocument` guarantees byte-level round-trip fidelity with the runtime parser.
- **Mutating `agent_settings.json` from skill CRUD vs. read-time filtering**: Skill creation and update do not rewrite `agent_settings.json`, avoiding a second writer for Admin assignments; `resolve_effective_agents` intersects `allowed_skills` and `available_skills` with the live skill catalog on read. Deletion is the cleanup exception: it prunes the deleted ID from persisted assignments so stale references are not retained.
- **Scanning `skills_dir` inside `Orchestrator::reconfigure` vs. dedicated `Orchestrator::reload_skills`**: `reconfigure` is also invoked during LLM Provider updates (`providers.rs`), where an unrelated malformed `SKILL.md` on disk must not block provider reconfiguration; separating `reload_skills` (backed by lenient discovery and per-run `Arc<SkillManager>` snapshots) isolates skill hot-reload from provider and agent settings updates.
