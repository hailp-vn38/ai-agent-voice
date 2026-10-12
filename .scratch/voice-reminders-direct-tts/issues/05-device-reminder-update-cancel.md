# 05: Sửa và hủy Reminder với CAS từ Admin Web

**What to build:** Admin sửa hoặc hủy lịch của Device, phát hiện thay đổi đồng thời và không mất lịch khi đổi Agent.

**Blocked by:** 04: Tạo và xem Reminder bền vững ở Device Detail.

Status: ready-for-agent

- [ ] PATCH/cancel conditional row revision đúng convention, stale edit trả conflict hiển thị trên UI; hủy là soft cancel, không xóa audit.
- [ ] Update/cancel và tăng đúng một Schedule Revision khi thay đổi pending cùng transaction; replay idempotent/rollback không tăng, khác payload conflict.
- [ ] Mutation keys caller-bound dùng chung create/update/cancel, retention không xóa key trước cửa sổ retry hỗ trợ.
- [ ] UI có edit/cancel confirmation, timezone/time validation và không tự nhận synced sau commit; API không cho chéo target Device.
- [ ] Reassignment giữ owner/Reminder; Device delete theo FK/retention hiện hữu có regression test, không vô tình xóa lịch khi đổi Agent.
- [ ] HTTP concurrency + restart + Web tests xác nhận hai writer không lost update, cancel replay an toàn và terminal lịch không bị sửa/phát lại sai.
