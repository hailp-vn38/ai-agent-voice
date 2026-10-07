# 17: Switch Template bằng hot runtime và quyền đúng

**What to build:** Speaker đã khóa switch sang Template được grant và có exact-set evidence; cold/denied switch giữ nguyên Template hiện tại.

**Blocked by:** 16: Thu hồi quyền và vô hiệu hóa WS bị ảnh hưởng.

**Status:** ready-for-agent

- [ ] Target dùng đúng admission-time DesiredProvider; preparation ngoài actor, không query latest configuration hoặc preload mọi candidate.
- [ ] Kiểm locked-speaker grant, epoch, target-space Voiceprint và exact qualification trước arm và apply; không fallback, auto-enroll hoặc downgrade.
- [ ] Hot Ready acquire được phép; target cold khi session giữ pipeline trả busy và không build, operator prepare trước session; failure giữ profile cũ.
- [ ] Apply tại normal boundary; giữ Prepared Template Profile leases và release profile cũ sau cleanup. Speaker identity giữ nguyên, lượt tiếp theo verify mới trên target.
- [ ] Public switch tests kiểm cùng/khác spaces, concurrent PATCH, revoked target, history/writer boundary, hot/cold outcomes và không có side effect trên switch thất bại.
