# 07: Nghiệm thu thật và cập nhật tài liệu

**What to build:** Xác nhận Wiki Conversation hoạt động trên vault và provider thật, đo các số liệu mà spec còn để ngỏ, rồi đưa tài liệu về đúng hiện trạng. Kết quả nghiệm thu được ghi lại; kịch bản nào chưa chạy thì để `NOT_RUN`, không suy ra từ test tự động.

Xem spec: [Kiểm tra tay](../spec.md#kiểm-tra-tay), [Tài liệu cập nhật khi triển khai](../spec.md#tài-liệu-cập-nhật-khi-triển-khai), [Further Notes](../spec.md#further-notes).

**Blocked by:** 03, 04, 05, 06

**Status:** ready-for-human

- [x] `NOT_RUN` — Hỏi, tải lại trang, hỏi câu nối tiếp và xác nhận ngữ cảnh còn đúng trên vault/provider thật
- [x] `NOT_RUN` — Đóng tab giữa câu trả lời rồi mở lại trên vault/provider thật
- [x] `NOT_RUN` — Tắt server giữa câu trả lời rồi khởi động lại trên vault/provider thật
- [x] `NOT_RUN` — Xóa một trang đã được trích dẫn rồi mở lại câu trả lời trên vault thật
- [x] `NOT_RUN` — Đo kích thước `graph_json` và `GET` chi tiết của một hội thoại dài; chưa có số đo thực tế để quyết định có cần route đồ thị riêng
- [x] Đã kiểm tra route xóa hội thoại ghi `admin_audit_events` như các route Admin tương đương; resource title và nội dung hội thoại không được ghi vào audit
- [x] Đã cập nhật tài liệu API, flow, phần phạm vi và lưu trữ trong tài liệu thiết kế; nêu rõ Backup Bundle không chứa hội thoại
- [x] ADR 0139 chuyển sang Accepted và ADR 0138 ghi rõ quyết định được sửa đổi
- [x] Spec còn chờ các bước nghiệm thu tay ở trên; giữ trạng thái `ready-for-human`

### Kết quả kiểm tra trong môi trường phát triển

Các test route/store, web và giới hạn lưu trữ đã chạy trong quá trình triển khai qua `scripts/agent-test.sh`; các kết quả đó không thay cho nghiệm thu vault/provider thật. Không chạy kịch bản nào trong danh sách `NOT_RUN` ở lượt triển khai này.


## Implementation result

Implemented and integrated on `feat/wiki-chat-conversations`. Focused server/web validation for this ticket passed through `scripts/agent-test.sh`; see the integration commits and test record in the implementation session.
