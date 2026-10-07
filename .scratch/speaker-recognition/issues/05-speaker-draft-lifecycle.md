# 05: Quản lý Speaker và draft enrollment

**What to build:** Admin tạo Speaker chưa có giọng, chọn provider và mở, xem, hủy hoặc tiếp tục draft từ giao diện Người nói.

**Blocked by:** 01: Tạo và kiểm tra Speaker Provider CAM++.

**Status:** in-progress (branch speaker/05)

- [ ] CRUD Speaker và draft có API/SQLite/UI đồng bộ, revisions riêng/ETag/CAS, provider/space/provenance pin và bounded quotas/TTL.
- [ ] Không cần bind Template trước; profile chưa enrolled không là candidate usable. Draft creation không thay Voiceprint active hoặc auto-grant/policy.
- [ ] Cancel/expiry/startup cleanup bounded; inspection/reconcile không expose audio/vector/digest. Provider references ngăn hard-delete bằng typed in-use response.
- [ ] Public HTTP và real SQLite restart tests kiểm auth, hai tab, quota, expiry, lost response, đúng FK/incarnation và compatible runtime repin; API docs cập nhật.
