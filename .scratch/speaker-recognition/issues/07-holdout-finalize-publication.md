# 07: Validate holdout và finalize Voiceprint

**What to build:** Admin hoàn tất wizard với 3–5 sample nhất quán và holdout mới, rồi thấy Voiceprint có hiệu lực cho admission mới mà không restart.

**Blocked by:** 06: Thu WAV trên web và lưu sample hợp lệ.

**Status:** ready-for-agent

- [ ] L2 normalize finite/dimension-correct vectors; all pair consistency, equal-weight centroid; holdout mới không duplicate exact PCM và phải pass Preliminary Calibration.
- [ ] Validate quyết định domain khác HTTP success; chỉnh sample đưa draft về collecting. Finalize CAS draft/Speaker/space revision và runtime/calibration pin.
- [ ] Atomic replace một space, terminalize draft và publish catalog nhất quán sau commit; không auto-grant/policy hoặc giữ bản sao vector dư trong draft.
- [ ] UI xử lý mismatch, validation expiry, lost response bằng GET, không auto-repeat finalize; finalized state/revision reconcile được.
- [ ] Public tests cover insufficient samples, wrong/different holdout, CAS races, rollback/commit-publish consistency, restart và không partial visibility; fixtures toán độc lập kiểm cosine/centroid.
