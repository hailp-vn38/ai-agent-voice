# 16: Thu hồi quyền và vô hiệu hóa WS bị ảnh hưởng

**What to build:** Admin re-enroll, purge hoặc thu hồi quyền; WS bị ảnh hưởng dừng dispatch mới và đóng1008, còn snapshot vẫn qualified/không revoke tiếp tục.

**Blocked by:** 08: Re-enroll, nhiều embedding spaces và purge, 15: Required xác minh mới ở từng voice turn.

**Status:** ready-for-agent

- [ ] Wire tất cả relevant Speaker/Agent/Template/provider/grant/policy handlers vào consistent mutation/catalog/security publication; không chỉ API Speaker mới.
- [ ] Re-enroll/disable/purge/reduce grants/unlink/contract/evidence revoke invalidates đúng dependencies; no mode downgrade, no profile hot reload.
- [ ] Recheck accept, mỗi LLM request/continuation/tool dispatch; stale pass không hồi quyền. Registry không strong-reference leak, mailbox failure dùng shutdown escape path.
- [ ] Valid additions/saved unqualified new set không tự close old qualified snapshot; additions không hot-expand rights. Already-dispatched work không rollback claim.
- [ ] UI mutation responses/dependencies rõ; production HTTP+WS race tests revoke ở từng dispatch boundary, writer/native cleanup, commit publication failure và unaffected sessions.
