# 08: Re-enroll, nhiều embedding spaces và purge

**What to build:** Admin đăng ký lại một space hoặc xóa rõ ràng dữ liệu giọng; profile, spaces khác và lịch sử không bị xóa ngầm.

**Blocked by:** 07: Validate holdout và finalize Voiceprint.

**Status:** ready-for-agent

- [ ] Re-enroll fail/cancel giữ giọng cũ; successful replace chỉ space đã chọn. Cùng space khác provider reuse; khác space không so vector.
- [ ] Replace/disable/purge publish security invalidation cho dependencies bị ảnh hưởng; subscriber session integration được hoàn thiện ở ticket16, không hot-expand snapshots.
- [ ] Purge explicit tất cả spaces/samples/drafts, giữ profile/grants/audit/history. Hard-delete Speaker/provider conditional, không cascade biometrics hoặc transcript.
- [ ] UI readiness theo selected provider/space; disable source provider không xóa vector dùng được trên instance compatible khác.
- [ ] HTTP/SQLite tests kiểm provenance FK, byte bounds, restart, failed replacement, concurrent revisions, purge data và conditional deletion.
