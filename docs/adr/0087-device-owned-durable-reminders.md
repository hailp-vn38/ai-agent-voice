# ADR 0087 — Device sở hữu Reminder bền vững, scheduler thuộc application

## Status

Accepted, 2026-10-26 (Proposed, 2026-10-12).

SQLite (`voice-agent.db`) là nguồn dữ liệu chính của Reminder, thuộc đúng một Device; Agent chỉ xác định phạm vi hiển thị và quyền. Chọn module persistence `DeviceNotifications` kết hợp tầng điều phối `ReminderCoordinator` dùng chung cho Admin API, builtin voice tools, WebSocket control plane và `reminder_scheduler` thay vì để mỗi adapter giữ lịch/SQL riêng hoặc đẩy nhắc việc từ `tasks.db`.

- Device đổi Agent/Template không chuyển hay xóa Reminder; danh sách theo Agent join các Device hiện thuộc Agent và tính trạng thái đồng bộ (`acked` / `pending` / `out_of_window` / `unsupported`) cho từng mục. Thời điểm đến hạn do server tính khi giao dịch tạo/sửa, lưu UTC; thời gian nhập mơ hồ cần timezone rõ.
- Create/update/cancel/completed/expired và tăng Schedule Revision trên Device phải cùng transaction; rollback, replay idempotent và attempt telemetry không tăng revision. PATCH/cancel dùng row revision CAS, độc lập với Schedule Revision.
- Mutation key bền vững có uniqueness `(caller_namespace, request_id, operation, device)` do server ràng buộc, ghi cùng mutation/response. Cùng payload replay kết quả, khác payload trả conflict; key và attempt có retention bounded, không xóa key trước cửa sổ retry được hỗ trợ. Một tool call mới không được suy ra là retry của call cũ.
- Production bootstrap chạy đúng một event-driven `reminder_scheduler`/process: ngủ chính xác đến `next_deadline_ms` (`due_at_ms`, `next_attempt_at_ms`, `lease_until_ms`, hoặc mốc hết hạn 24h) và thức dậy tức thì qua `tokio::sync::Notify` khi có mutation, thiết bị kết nối, hoặc kết thúc lượt nói (`SessionActor` turn boundary). Không quét mù mỗi 1 giây, không spawn theo router clone, WS hoặc mỗi Reminder; DB I/O không block `SessionActor`.

V1 chỉ có lịch một lần, một Device đích. Lease/Delivery Attempt tách khỏi lifecycle `pending/completed/cancelled/expired` để phục hồi sau crash mà không biến dispatch thành trạng thái nghiệp vụ lâu dài. Migration tuân theo [ADR-0063](0063-forward-only-schema-migrations.md); SQLite commit chỉ có nghĩa đã lưu, chưa đồng bộ hay phát.

Nguồn: [Guide §2–4, §10, §13–16](../voice_reminders_direct_tts_implementation_guide.md#2-domain-model-và-những-bất-biến-bắt-buộc).
