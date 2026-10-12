# 09: Quản lý Reminder bằng giọng nói với quyền Agent

**What to build:** Physical Voice Device tạo/list/sửa/hủy Reminder qua builtin tools và chỉ xác nhận sau DB commit, với quyền Admin cấp rõ ràng.

**Blocked by:** 06: Đồng bộ lịch qua Voice WS và hiển thị Cache ACK.

Status: ready-for-agent

- [ ] Admin Agent GET/PATCH + revision và Web setting quản lý reminders_enabled; global feature và Agent grant default false, advertisement ở admission và live DB mutation guard.
- [ ] Bốn builtin tools reserved-name chống shadowing, async bounded executor; Physical Device self-target, MCP false vẫn dùng builtin có phép, speaker/MCP grant không cấp quyền.
- [ ] Câu nhắc mười phút tạo một row đúng server clock; list query dữ liệu thực, update/cancel CAS/idempotency, tool round không nói đã lưu trước transaction thành công.
- [ ] Conditional reminder policy/bounded escaped time/timezone/target/capability context, no full list/message/pending_count/new placeholder; legacy Templates và 96 KiB cap giữ đúng.
- [ ] Tool continuation reuse cùng System snapshot; revoke giữa session chặn writes mà không hot reload Effective Session Profile/catalog.
- [ ] WS + Admin + Web tests chứng minh four-tool flow/auth/revoke/time clarification/tool errors/replay và saved khác cache ACK; không cần online delivery để hoàn tất ticket này.
