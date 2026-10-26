# ADR 0091 — Reminder dùng quyền riêng và target được server xác thực

## Status

Accepted, 2026-10-26 (Proposed, 2026-10-12; mở rộng của ADR-0032, ADR-0052 và ADR-0084).

Bộ 4 Reminder builtin tools (`server_create_reminder`, `server_list_reminders`, `server_update_reminder`, `server_cancel_reminder`) chỉ được quảng bá khi quyền Agent `reminders_enabled` bật, mặc định deny; khi được cấp quyền, `SessionActor` ẩn các tool `reminder_*` cũ của `TaskToolSuite` và định tuyến mọi lệnh nhắc việc về `SessionReminderTools` (`ReminderCoordinator` / `voice-agent.db`). Executor kiểm tra lại trực tiếp `agents.reminders_enabled` trong SQLite trước mỗi lần thực thi để thu hồi quyền có hiệu lực ngay với cả Voice Session đang mở. Chọn quyền riêng và target server-bound thay vì suy quyền từ feature flag client, speaker identity, MCP grant hoặc Device ID do LLM tạo.

- Physical Voice Session chỉ thao tác Device đã admission (`source = 'voice'`). Studio Assistant (`source = 'web_assistant'`) phải có physical/enabled target thuộc cùng Agent được Admin chọn qua `PUT /api/admin/system/assistant-device/reminder-target`, gắn với `session_id` hiện thời và TTL 15 phút (`REMINDER_TARGET_TTL`); thiếu/stale target hoặc thiết bị bị vô hiệu hóa trả `target_selection_required`. Virtual Device không là RTC wake target. Admin CRUD lấy target từ route và auth hiện hữu (`source = 'web'`); `features.mcp=false` không vô hiệu hóa builtin tools được cấp quyền.
- Client Direct TTS cần feature opt-in và grant server-side `direct_tts.speak`, validate/rate/admission; payload không tự gán Device/provider/source. Admin dùng route riêng đến active Device. Các message mới bắt buộc đúng session ID; trả typed accepted/rejected cho request mới hợp lệ của client opt-in, trong khi malformed/unknown application messages và legacy flow vẫn theo ADR-0032.
- Tool round giữ lời khẳng định đến khi DB commit thành công; trả lời phân biệt saved, cache ACK và played. UI lấy live WS presence từ registry, không suy từ `devices.enabled`.
- Compose đúng một System message/turn với conditional `<reminder_policy>` và bounded escaped `<reminders_info>` (thời gian/timezone/target/capability); không nhúng pending count, full list hoặc reminder text. List dùng tool query. Không thêm placeholder hay rewrite stored Templates; giữ cap 96 KiB và snapshot nguyên vẹn qua tool continuation theo ADR-0084.

Effective Session Profile/provider snapshot vẫn immutable theo ADR-0052; live permission/target validation là trust boundary riêng, không sửa nóng profile/catalog. Target authorization không được thay bằng tên Device trong hội thoại.

Nguồn: [Guide §5.4, §8–11](../voice_reminders_direct_tts_implementation_guide.md#8-llm-tools-và-system-prompt).
