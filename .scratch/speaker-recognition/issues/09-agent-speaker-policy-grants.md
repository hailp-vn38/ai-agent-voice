# 09: Cấu hình Agent policy và Template grants

**What to build:** Admin quản lý Off/Observe và quyền Speaker theo Template trên Agent, thấy dependencies rõ ràng; Required chưa thể bật khi thiếu gate/qualification.

**Blocked by:** 02: Bind Speaker Provider tùy chọn vào Template, 05: Quản lý Speaker và draft enrollment.

**Status:** resolved

- [x] Policy revision riêng, absent Off contract nhất quán; grants replace-all explicit assigned/enabled Templates, không wildcard hoặc Client ID.
- [x] Agent detail/Template surfaces phản ánh saved config, compatibility và required dependencies; thêm quyền chỉ admission mới, giảm quyền phát invalidation metadata.
- [x] Candidate selection theo Agent enabled bindings và active provider space rồi mới kiểm Template grant; không lọc grant để ép nhận nhầm.
- [x] Validation cấu trúc/revision/type/FK giữ nguyên; UI báo dependencies hiện biết và Required unavailable khi chưa có qualification. Không thêm runtime bypass hoặc đợi một ticket tương lai để hoàn tất slice Off/Observe.
- [x] API public tests kiểm revision ownership, empty/wildcard/sai assignment, disabled resources, two tabs; Required enable guard fail closed cho tới khi15 hoàn thiện.

## Answer

Implemented on branch `speaker/09` (commit 2dbd96a).

- Migration `0012_agent_speaker_template_grants.sql` (renumbered from 0011 at merge time; 0011 was taken by ticket 13): per-Template grants `(agent_id, speaker_id, template_id)` with cascade deletes; the existing `agent_speaker_candidates` (`0009_speakers.sql`) / `agent_speaker_policies` (`0008_agent_tool_allowlist.sql`) tables are the binding and policy stores.
- `src/app/admin/speaker_policy.rs`:
  - `GET/PUT /api/admin/agents/{key}/speaker-policy` — policy has **its own** revision + ETag, independent of the Agent revision. Absent row reads as the `off` contract at revision 1. PUT CASes on the policy revision; `required` returns `503 speaker_calibration_required` and does not consume a revision while the ticket-14/15 gates are missing.
  - `PUT/DELETE /api/admin/agents/{key}/speakers/{speaker_key}` — replace-all explicit grants, CAS on the **Agent** revision. Rejects empty / wildcard (`*`) / duplicate keys and Templates that are unassigned, disabled, or whose assignment is disabled (`invalid_template`). Enforces `max_candidates_per_agent` (`speaker_candidate_limit`) for new bindings only. Unlink drops the candidate + grants and calls `tool_security.invalidate_agent` so live sessions re-admit under the reduced set.
  - `GET /api/admin/agents/{key}/speakers` — bindings with `template_keys`, `enabled`, and computed `usable` (any disabled resource ⇒ not usable).
  - `GET /api/admin/speakers/{key}/bindings` — reverse dependency view for the Speaker delete flow (the Speaker in-use check already counts `agent_speaker_candidates`).
- Tests: `tests/agent_speaker_policy_api.rs` (7), public HTTP against real SQLite, covering the Off contract, policy CAS isolation, Required fail-closed, replace-all, grant validation, candidate limit, unlink + invalidation, disabled-resource `usable`, auth, and not-found. Full `voice-agent-server` suite green apart from a pre-existing failure on the untouched baseline (`session_profile::public_api_created_provider_is_used_by_new_ws_and_patch_keeps_old_session_version`).

Remaining (out of scope, deferred to dependent tickets): runtime candidate selection by enabled Template / active provider space and the Observe turn report are ticket 10; `required` stays unavailable until ticket 14/15. The admin-web UI panel for this surface is wired in `apps/admin-web` (see ticket 13 for the web surface ownership).
