# 08: Re-enroll, nhiều embedding spaces và purge

**What to build:** Admin đăng ký lại một space hoặc xóa rõ ràng dữ liệu giọng; profile, spaces khác và lịch sử không bị xóa ngầm.

**Blocked by:** 07: Validate holdout và finalize Voiceprint.

**Status:** resolved

- [x] Re-enroll fail/cancel giữ giọng cũ; successful replace chỉ space đã chọn. Cùng space khác provider reuse; khác space không so vector.
- [x] Replace/disable/purge publish security invalidation cho dependencies bị ảnh hưởng; subscriber session integration được hoàn thiện ở ticket16, không hot-expand snapshots.
- [x] Purge explicit tất cả spaces/samples/drafts, giữ profile/grants/audit/history. Hard-delete Speaker/provider conditional, không cascade biometrics hoặc transcript.
- [x] UI readiness theo selected provider/space; disable source provider không xóa vector dùng được trên instance compatible khác.
- [x] HTTP/SQLite tests kiểm provenance FK, byte bounds, restart, failed replacement, concurrent revisions, purge data và conditional deletion.

## Answer

Implemented on branch `speaker/08` (commit `00f3d08`), based on `integration/speaker-recognition`.

**Schema (`migrations/0009_speakers.sql`)** — ticket 08 is a pre-release branch, so the constraint bug
is fixed in place rather than stacked into a new migration.
- The draft `status` CHECK now admits `'expired'`, and the terminal-state CHECK accepts
  `committed|expired`. Previously both rejected `'expired'`, so
  `cleanup_expired_drafts`' UPDATE raised a CHECK violation that the sweep swallowed — expired
  drafts (and their captured audio) were never tombstoned. This is why the re-enroll
  "different draft id, same space" invariant was also unenforced across a restart.
- The expiry sweep is now `terminal_at IS NULL`-driven (partial index updated to match); no draft
  ever sits in `'collecting'` with a `terminal_at`, so the terminal invariant holds for both
  `'committed'` and `'expired'`.

**API (`src/app/admin/speakers.rs`, route in `mod.rs`)**
- `publish_catalog_revision(&mut tx)` extracted from `finalize`; **replace** (finalize), **disable**
  (patch) and **purge** all bump it. This is the security-invalidation signal.
- `POST /speakers/{key}/voiceprint/purge`: requires `{"confirm":"PURGE_SPEAKER_VOICEPRINT"}` (else
  `confirmation_required`) and `If-Match` (else `revision_conflict`, audited). One transaction
  deletes every `speaker_enrollment_samples` row for the speaker's drafts, then the drafts, then
  all `speaker_voiceprints`, bumps the speaker revision and the catalog revision, and writes one
  `purge_voiceprint` audit event. Profile, grants and audit trail survive; transcripts are never
  touched. Returns the updated speaker with a fresh ETag.
- Disabling a speaker (`patch`, `enabled: false`) now publishes a catalog invalidation.
- Fix: deleting an in-use speaker returned `resource_in_use`, a code no client can translate; it is
  now `speaker_in_use`, matching `apps/admin-web/src/api/errors.ts`.
- `delete` already blocks non-empty biometrics without cascading (voiceprints/candidates/drafts),
  so hard-delete stays conditional as the ticket requires.

**UI (`apps/admin-web`)**
- `speakersApi.purgeVoiceprint(key, revision)` + a `ConfirmDialog`-driven "Purge voice" action shown
  only when a voiceprint exists, wired through the i18n catalog (en/vi).
- Provider-disable does not erase vectors: `PATCH /providers/{key}` never touches
  `speaker_voiceprints`; the published vector survives and stays usable on a compatible instance.

**Tests**
- `cargo test -p voice-agent-server --test speaker_enrollment_api` → 19 passed. New cases:
  re-enroll replaces one space and re-publishes the catalog revision (durable across restart);
  purge removes every sample + both drafts, bumps speaker/catalog revisions, keeps the profile and
  one audit event, and leaves the speaker hard-deletable; purge confirmation/CAS/404 guards leave
  the voiceprint intact; disabling a speaker moves the catalog revision; disabling the enrollment
  provider keeps the vector.
- `cargo test -p voice-agent-server --lib` → 200 passed incl. a new sweep unit test that fails if
  the `status` CHECK or the sweep query regresses.
- `cargo fmt --check` clean; `cargo clippy --all-targets` shows only the two pre-existing warnings.
- `apps/admin-web`: `vue-tsc` clean, `vitest run` → 48 passed (new purge-client case).

## Remaining gaps

- `replace`/`disable`/`purge` publish by bumping the single-row `speaker_catalog` revision; there is
  still no in-process admission snapshot or WS/epoch propagation. That subscriber/session wiring is
  explicitly ticket 16's job, so ticket 08 only has to move the revision.
- The `session_profile` test
  `public_api_created_provider_is_used_by_new_ws_and_patch_keeps_old_session_version` fails (202 vs
  200) on `integration/speaker-recognition` **before** this branch; confirmed by stashing the
  working tree. Unrelated to this change.
- A "failed replacement keeps the old voiceprint" test is not added: ticket 07's `finalize`
  already rolls back on any failure and its suite covers the CAS rollback; ticket 08 adds no new
  failure path to `finalize`.
- `cleanup_expired_drafts` propagation is verified by a synchronous unit test, not an HTTP test —
  the startup sweep races the test harness's readiness probe, making an end-to-end assertion flaky.
- Purge is per-speaker and has no dry-run/preview; it is a confirmed destructive action by design.
