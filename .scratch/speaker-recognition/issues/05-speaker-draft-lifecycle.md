# 05: Quản lý Speaker và draft enrollment

**What to build:** Admin tạo Speaker chưa có giọng, chọn provider và mở, xem, hủy hoặc tiếp tục draft từ giao diện Người nói.

**Blocked by:** 01: Tạo và kiểm tra Speaker Provider CAM++.

**Status:** resolved

- [x] CRUD Speaker và draft có API/SQLite/UI đồng bộ, revisions riêng/ETag/CAS, provider/space/provenance pin và bounded quotas/TTL.
- [x] Không cần bind Template trước; profile chưa enrolled không là candidate usable. Draft creation không thay Voiceprint active hoặc auto-grant/policy.
- [x] Cancel/expiry/startup cleanup bounded; inspection/reconcile không expose audio/vector/digest. Provider references ngăn hard-delete bằng typed in-use response.
- [x] Public HTTP và real SQLite restart tests kiểm auth, hai tab, quota, expiry, lost response, đúng FK/incarnation và compatible runtime repin; API docs cập nhật.

## Answer

Implemented on branch `speaker/05` (commit a6365a8), rebased onto current `integration/speaker-recognition`, migration renumbered `0008_speakers.sql` → `0009_speakers.sql` (0008 was taken by ticket 11) and merged.

- Migration `0009_speakers.sql`: `speaker_profiles`, `speaker_voiceprints`, `speaker_voiceprint_samples`, `speaker_enrollments`, `speaker_enrollment_samples`; bounded quotas (min 3 / max 5 samples, max 16 open drafts), TTL/expiry indexes, FK/incarnation keying.
- `src/database/speakers.rs`; admin API `src/app/admin/speakers.rs` (`/api/admin/speakers` CRUD + draft enrollment lifecycle, ETag/`If-Match` CAS, typed `speaker_in_use` 409 on delete, draft pins provider/revision/embedding-space/provenance).
- Frontend: `api/speakers.ts`, `api/types/speakers.ts`, `views/SpeakersView.vue`, `pages/speakers/SpeakerDetailPage.vue` (two tabs), recorder flow wired into router/navigation/i18n.
- Tests: `tests/speaker_enrollment_api.rs` (auth, two tabs, quota, expiry, lost-response idempotency, FK/incarnation, compatible vs incompatible repin, `speaker_in_use`); migration-preservation test; frontend `api/speakers.test.ts` (3). All green; `vue-tsc --noEmit` clean.
- Docs: `docs/api/tool-allowlist.md`-adjacent speaker API docs + Postman collection entries.

Remaining (deferred to ticket 06+): real sample capture/finalization from browser audio is ticket 06/07/08.
