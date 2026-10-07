# 14: Reload calibration và kiểm exact candidate sets

**What to build:** Operator reload catalog deployment; Admin thấy profile preliminary/qualified và dependencies theo exact set, trong khi set mới thiếu evidence vẫn lưu được.

**Blocked by:** 07: Validate holdout và finalize Voiceprint, 09: Cấu hình Agent policy và Template grants.

**Status:** resolved

- [x] Admin Bearer reload fixed configured source, không fields/path/URL override; Web chỉ inspection, không reload/edit/bypass. Token holder vẫn có thể gọi API.
- [x] Validate whole catalog rồi publish/invalidate nhất quán; failed reload giữ catalog cũ, file-edit alone không effect; không model/runtime/fullTOML reload.
- [x] Evidence pin actual Agent/Template scoring sets, candidate identities/Voiceprint revisions, space/scoring/preprocessing/audio/execution/load; subset không tự qualified.
- [x] Multiple exact evidence sets; adding set/catalog generation không revoke old snapshot; explicit removal/contract change phát targeted invalidation.
- [x] Valid domain mutation không409 chỉ thiếu evidence; giữ Required mode, UI saved-but-unavailable/dependencies; enable Required vẫn gated. Public reload/CAS/publication/scope tests.

## Answer

Implemented on branch `speaker/14` (commit `b4c5a2b`), based on `integration/speaker-recognition`.

**Schema (`migrations/0014_speaker_calibration_catalog.sql`)**
- Single-row `speaker_calibration_catalog` (id = 1) holds the published `calibration_revision`, source digest, validated `profiles_json`, and `published_at`; no row means only the built-in preliminary calibration applies.
- `speaker_calibration_evidence` is keyed by `(calibration_revision, candidate_set_digest)` and stores the exact `candidate_set_json` snapshot plus its `report_ref`.

**Config (`src/config/mod.rs`)**
- `[speaker_recognition] calibration_source: Option<PathBuf>` — the fixed deployment source. It is never accepted per request; absent means reloads report `speaker_catalog_unavailable`.

**Catalog + reload (`src/app/admin/speaker_calibration.rs`, route in `mod.rs`)**
- `POST /api/admin/speaker-recognition/reload`, behind the existing Admin bearer. It accepts only an empty body or `{}`; any field/path/URL override is refused with `400 calibration_reload_override_not_allowed`.
- The whole file is parsed and validated (revision/profile shape, status, thresholds, duplicate/unknown Agent references) before any write, so a malformed file or unknown Agent returns `422 calibration_invalid` and the previously published catalog stays live. Editing the source file alone has no effect until a reload.
- Publication is one transaction: upsert the catalog row, delete only the evidence entries for the published revision that the file no longer lists, and upsert the rest. Other revisions' evidence is untouched, so a new voiceprint catalog generation or a new evidence set does not revoke old snapshots; a dropped entry or a new calibration revision does invalidate exactly the affected set.
- The candidate-set digest is canonical JSON over the Agent's candidate identities, per-Template grants, and voiceprint spaces/revisions/provider/calibration pins, plus the pinned preprocessing/audio/scoring contract. The voiceprint catalog generation is deliberately excluded, matching ADR 0081.

**Summary + policy (`speakers.rs`, `speaker_policy.rs`)**
- `GET /api/admin/speaker-recognition` now carries a `calibration` object (`revision`, `status`, `published_at`, `profiles`, `evidence_sets`).
- `required_blockers` is now computed from real qualification (evidence for the Agent's exact current set under the published revision with a qualified profile). Valid domain mutations still save without evidence; only enabling `required` is gated. Because the fresh-turn gate (ticket 15) is not built, `required` remains blocked by `speaker_fresh_turn_verification_required` once calibration qualifies — a deliberate fail-closed handoff.

**Tests (`tests/speaker_calibration_api.rs`)**
- 6 public HTTP + real SQLite tests: bearer scope, fixed source vs override, no-source 503, invalid reload keeps the old catalog, qualified reload clears the calibration blocker but keeps `required` gated, targeted revocation of only the removed exact set, and new-revision invalidation with a still-saveable domain mutation.
- `cargo test -p voice-agent-server` → 199 lib passed; new suite 6 passed; `agent_speaker_policy_api`, `speaker_enrollment_api`, `admin_api` all green. `cargo fmt --check` and `cargo clippy -p voice-agent-server` clean for the touched files.

## Remaining gaps

- Affected-session closure on invalidation is not wired: reload records revocation and the policy gate re-blocks, but no live WS is closed yet (ticket 16 owns that, on top of the recognition pipeline in tickets 10–13).
- The runtime does not yet consume the reloaded catalog for scoring; the enrollment/finalize path still pins `PRELIMINARY_CALIBRATION` (tickets 10–13/17 swap it in).
- The digest pins the contract as a fixed `pcm16-mono16k-v1` constant rather than reading live provider/audio descriptors; tighten if the pipeline exposes them.
- `session_profile::public_api_created_provider_is_used_by_new_ws_and_patch_keeps_old_session_version` fails on a clean `HEAD` (pre-existing, unrelated); verified by stashing this branch's changes.
