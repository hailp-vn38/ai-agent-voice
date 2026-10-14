# 06: Đồng bộ lịch qua Voice WS và hiển thị Cache ACK

**What to build:** Admin thay đổi lịch, Device opted-in nhận hint/lấy snapshot/ACK và Device Detail hiển thị đúng từng mục đã cache.

**Blocked by:** 02: Thử Direct TTS trên Device từ Admin Web; 05: Sửa và hủy Reminder với CAS từ Admin Web.

Status: claimed

- [ ] Sau create/update/cancel commit gửi sync_required, coalesce giữ revision cao nhất; reconnect luôn sync_request, metadata/control không gửi client cũ.
- [ ] Snapshot nhất quán tối đa 64 pending, sort due_at/id, snapshot ID/revision/server time/has_more/next sync; contracts cho terminal mutations dùng cùng publisher.
- [ ] Server validate authenticated Device/current session/snapshot token/revision và membership đã gửi; ACK cũ không làm lùi cache revision.
- [ ] GET Device reminders hợp nhất schedule summary, actual presence/capability và sync states; UI phân biệt pending/acked/out_of_window/unsupported, không endpoint sync riêng.
- [ ] Public WS/HTTP tests có 0/1/64/65/500 items, hint thất lạc/reconnect, request–ACK mutation race, stale token/session và rollback không push.
- [ ] Protocol playback harness ACK cache theo contract; không tuyên bố NVS/hardware verified hoặc giả playback ACK từ cache ACK.
