# 07: Validate holdout và finalize Voiceprint

**What to build:** Admin hoàn tất wizard với 3–5 sample nhất quán và holdout mới, rồi thấy Voiceprint có hiệu lực cho admission mới mà không restart.

**Blocked by:** 06: Thu WAV trên web và lưu sample hợp lệ.

**Status:** resolved

- [x] L2 normalize finite/dimension-correct vectors; all pair consistency, equal-weight centroid; holdout mới không duplicate exact PCM và phải pass Preliminary Calibration.
- [x] Validate quyết định domain khác HTTP success; chỉnh sample đưa draft về collecting. Finalize CAS draft/Speaker/space revision và runtime/calibration pin.
- [x] Atomic replace một space, terminalize draft và publish catalog nhất quán sau commit; không auto-grant/policy hoặc giữ bản sao vector dư trong draft.
- [x] UI xử lý mismatch, validation expiry, lost response bằng GET, không auto-repeat finalize; finalized state/revision reconcile được.
- [x] Public tests cover insufficient samples, wrong/different holdout, CAS races, rollback/commit-publish consistency, restart và không partial visibility; fixtures toán độc lập kiểm cosine/centroid.

## Answer

Implemented on branch `speaker/07` (commit `07626b1`), based on `integration/speaker-recognition`.

**Math (`crates/voice-agent-server/src/audio/enrollment.rs`)**
- `cosine`, `pairwise_min_cosine`, `centroid` (equal-weight mean of normalized vectors, re-normalized), `decode_embedding`.
- `Calibration` + `PRELIMINARY_CALIBRATION` (`vi_esp32_pilot_v1`, accept 0.45, consistency 0.50) with a `ponytail:` note that ticket 14 swaps in the reloaded deployment catalog.
- Independent hand-computed unit fixtures cover cosine, pairwise floor, and centroid (including cancel-to-zero and dimension mismatch).

**Schema (`migrations/0013_speaker_holdout_finalize.sql`)**
- Draft validation provenance columns (`validation_status`, `validation_revision`, `validation_calibration_revision`, `validation_runtime_id`, `validation_provider_revision`, `holdout_digest`).
- `speaker_enrollment_samples.pcm_digest` and `speaker_voiceprints.calibration_revision`.
- Single-row `speaker_catalog` generation bumped inside the finalize transaction.

**API (`src/app/admin/speakers.rs`, routes in `mod.rs`)**
- `POST .../enrollments/{id}/validate`: quality gate, exact-PCM duplicate rejection, worker embedding, holdout-vs-centroid score, consistency floor. Decision (`passed`/`failed`/`inconsistent`/`ambiguous`) is HTTP 200; only transport/quality/conflict are 4xx/5xx. Bumps the draft revision and pins calibration/runtime/provider revision.
- `POST .../enrollments/{id}/finalize`: CAS on draft `If-Match`, live Speaker revision, and selected-space Voiceprint revision; requires a passing validation for the current revision and the pinned calibration; recomputes the centroid; then one transaction bumps `speaker_catalog`, upserts only this embedding space (`browser_validation_status = passed`), and terminalizes the draft. No grants/policy touched, no second vector copy stored in the draft.
- `draft_resource` exposes the `validation` object with `valid_for_current_revision`; sample PUT/DELETE clear the decision.
- `create_draft` now pins `base_voiceprint_revision` so re-enrollment CAS is meaningful.

**UI (`apps/admin-web`)**
- Types/API client for validate + finalize; `SpeakerDetailPage.vue` shows the decision and a validate/finalize flow, refetches the draft on error (lost-response reconcile), and labels a passed voiceprint "Đăng ký đạt, chưa kiểm trên ESP32". `min_samples` from the summary gates the validate button.

**Tests**
- `cargo test -p voice-agent-server --lib` → 199 passed, 0 failed (1 ignored).
- `cargo test -p voice-agent-server --test speaker_enrollment_api` → 13 passed, 0 failed. New cases: validate gate + publish, different-speaker failure, exact-duplicate rejection, sample-edit expiry, stale draft/speaker CAS, minimum sample count, restart durability.
- `cargo fmt --check`, `cargo clippy --all-targets` clean for the touched files.
- `apps/admin-web`: `npm run typecheck` clean, `npm test` → 47 passed.

## Remaining gaps

- Calibration thresholds are the pinned pilot constants, not a reloaded deployment catalog (ticket 14 / ADR 0077 owns that). The finalize response reports the preliminary revision.
- The admin `transport` middleware caps every mutation body at a hardcoded 256 KiB (`MAX_BODY`), so holdouts/clips between ~8.2 s and `max_clip_ms` (10 s) are rejected despite `max_audio_body_bytes` allowing up to 4 MiB. Pre-existing from ticket 05; not changed here to keep the diff scoped.
- No fault-injection test for a mid-finalize rollback (the catalog bump happens inside the finalize transaction, and a failed pre-transaction CAS is asserted not to advance it; only mid-transaction failure is untested).
- The catalog is a monotonic generation counter; no in-process admission snapshot is built yet (recognition pipeline lands in tickets 10–13).
