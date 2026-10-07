# 06: Thu WAV trên web và lưu sample hợp lệ

**What to build:** Admin ghi một sample bằng microphone browser và thấy slot accepted hoặc lỗi quality có thể sửa trong draft.

**Blocked by:** 04: Cold preparation dùng admission nguyên tử, 05: Quản lý Speaker và draft enrollment.

**Status:** resolved

- [x] Recorder AudioWorklet/downmix/resample tạo thật PCM16 mono16k; clips5–10s, auto-stop10s, bounded RAM; HTTPS/localhost requirements và CORS hiện có rõ ràng.
- [x] Raw WAV route cap512KiB toàn body/chunked, parser cap12s, reject encoding/format giả; JSON cap256KiB không thay. Reserve bounded upload/native enrollment capacity.
- [x] Preliminary Calibration deployment profile và pinned preprocessing cung cấp quality/window rules; native inference qua exact manager/worker, không giữ transaction khi compute.
- [x] Slot1–5 upload sequential CAS; quality fail không mutate; replace/remove làm stale validation mất hiệu lực. Bounded metadata/embedding storage, không raw audio retention.
- [x] Cleanup tracks/nodes/AudioContext/fetch/object URLs khi cancel/navigation; không browser persistence. Browser encoding tests + HTTP WAV/mutation/cancel/timeout tests cùng slice.

## Answer

Implemented on branch `speaker/06` (commit a0c7769), rebased onto current `integration/speaker-recognition` and merged.

- Browser: `apps/admin-web/src/lib/wav.ts` (PCM16 mono 16 kHz downmix/resample/WAV encoder), `composables/useMicrophoneRecorder.ts` (AudioWorklet + fallback, 5–10s clips, auto-stop at 10s, bounded RAM, full cleanup on cancel/navigation, no browser persistence).
- Server: `src/audio/enrollment.rs` (WAV parser capped at 12s, rejects fake encoding/format); `src/app/admin/speakers.rs` sample routes (`PUT`/`DELETE .../samples/{slot}`) with a 512 KiB whole-body cap (chunked-safe, does not trust Content-Length), JSON routes keep 256 KiB; reserves upload/native capacity via ticket 04's admission hook and computes outside the DB transaction.
- Native inference via the existing manager/worker path; quality rules from the Preliminary Calibration profile + pinned preprocessing.
- Sequential slot CAS (1–5); a quality failure does not mutate state; replace/remove invalidates prior validation. Bounded metadata/embedding storage, no raw audio retention.
- Tests: `tests/speaker_enrollment_api.rs` (+469 lines: WAV happy path, over-cap incl. chunked, fake-format, duration cap, CAS conflict, quality-fail no-mutate, replace invalidates, cancel/timeout no partial, no raw audio), frontend `lib/wav.test.ts` + `api/speakers.test.ts` (9). All green; `vue-tsc --noEmit` clean.

Remaining (deferred to ticket 07): holdout validation + voiceprint finalize/publication.
