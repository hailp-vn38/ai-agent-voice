# ADR 0087 — Device sở hữu Reminder bền vững, scheduler thuộc application

## Status

Proposed, 2026-10-12. Theo implementation guide v1.1; chưa triển khai hoặc nghiệm thu.

SQLite là nguồn dữ liệu chính của Reminder, thuộc đúng một Device; Agent chỉ xác định phạm vi hiển thị và quyền. Chọn một module application-owned `DeviceNotifications` dùng chung cho Admin, builtin tools và WS thay vì để mỗi adapter giữ lịch/SQL riêng hoặc dùng LLM làm scheduler.

- Device đổi Agent/Template không chuyển hay xóa Reminder; danh sách theo Agent join các Device hiện thuộc Agent. Thời điểm đến hạn do server tính khi giao dịch tạo/sửa, lưu UTC; thời gian nhập mơ hồ cần timezone rõ.
- Create/update/cancel/completed/expired và tăng Schedule Revision trên Device phải cùng transaction; rollback, replay idempotent và attempt telemetry không tăng revision. PATCH/cancel dùng row revision CAS, độc lập với Schedule Revision.
- Mutation key bền vững có uniqueness `(caller_namespace, request_id, operation, device)` do server ràng buộc, ghi cùng mutation/response. Cùng payload replay kết quả, khác payload trả conflict; key và attempt có retention bounded, không xóa key trước cửa sổ retry được hỗ trợ. Một tool call mới không được suy ra là retry của call cũ.
- Khi bật tính năng, production bootstrap chạy đúng một scheduler/process: scan bounded, notify khi mutation/reconnect, recovery lease lúc startup và shutdown có cancellation/join. Không spawn theo router clone, WS hoặc mỗi Reminder; DB I/O không block SessionActor.

V1 chỉ có lịch một lần, một Device đích. Lease/Delivery Attempt tách khỏi lifecycle `pending/completed/cancelled/expired` để phục hồi sau crash mà không biến dispatch thành trạng thái nghiệp vụ lâu dài. Migration tuân theo [ADR-0063](0063-forward-only-schema-migrations.md); SQLite commit chỉ có nghĩa đã lưu, chưa đồng bộ hay phát.

Nguồn: [Guide §2–4, §10, §13–16](../voice_reminders_direct_tts_implementation_guide.md#2-domain-model-và-những-bất-biến-bắt-buộc).
