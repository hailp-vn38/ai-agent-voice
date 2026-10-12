# 10: Studio Assistant chọn Device rồi quản lý Reminder

**What to build:** Admin chọn physical Device trong Floating Chat, bind vào current session và hội thoại thao tác đúng target đã xác thực.

**Blocked by:** 09: Quản lý Reminder bằng giọng nói với quyền Agent.

Status: ready-for-agent

- [ ] Admin-authenticated target binding endpoint kiểm tra active Studio instance, physical/enabled Device/scope; server giữ TTL ngắn, target không là payload quyền do model chọn.
- [ ] Web selector loading/error/clear/reconnect states, missing/stale/mismatch trả target_selection_required; Virtual Device không là RTC target.
- [ ] Four builtin tools lấy server-bound target và live quyền; giọng nhắc tên Device/forged device_id không thay binding hoặc tăng quyền.
- [ ] Reconnect/session replacement/TTL expiry/Device disable làm binding không còn dùng được; conversation confirmation phân biệt saved/synced/played.
- [ ] Public Admin + WS + Web tests chứng minh unauthorized binding, wrong session, valid selection, target changes giữa tool dispatch và stale/revoked permissions.
