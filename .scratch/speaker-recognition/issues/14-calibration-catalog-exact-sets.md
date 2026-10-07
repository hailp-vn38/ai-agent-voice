# 14: Reload calibration và kiểm exact candidate sets

**What to build:** Operator reload catalog deployment; Admin thấy profile preliminary/qualified và dependencies theo exact set, trong khi set mới thiếu evidence vẫn lưu được.

**Blocked by:** 07: Validate holdout và finalize Voiceprint, 09: Cấu hình Agent policy và Template grants.

**Status:** ready-for-agent

- [ ] Admin Bearer reload fixed configured source, không fields/path/URL override; Web chỉ inspection, không reload/edit/bypass. Token holder vẫn có thể gọi API.
- [ ] Validate whole catalog rồi publish/invalidate nhất quán; failed reload giữ catalog cũ, file-edit alone không effect; không model/runtime/fullTOML reload.
- [ ] Evidence pin actual Agent/Template scoring sets, candidate identities/Voiceprint revisions, space/scoring/preprocessing/audio/execution/load; subset không tự qualified.
- [ ] Multiple exact evidence sets; adding set/catalog generation không revoke old snapshot; explicit removal/contract change phát targeted invalidation.
- [ ] Valid domain mutation không409 chỉ thiếu evidence; giữ Required mode, UI saved-but-unavailable/dependencies; enable Required vẫn gated. Public reload/CAS/publication/scope tests.
