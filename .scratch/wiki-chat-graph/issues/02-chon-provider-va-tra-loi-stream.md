# 02: Chọn provider và trả lời stream

**What to build:** Admin chọn một chat LLM Provider cho tính năng Hỏi wiki trong Settings, rồi `POST /knowledge/ask` stream được câu trả lời từ provider đó, chưa có tool. Ticket này kiểm chứng sớm rủi ro lớn nhất của spec: lấy được `LlmProvider` có tool từ `AppState` ở cả hai chế độ runtime. Xem spec [wiki-chat-graph](../spec.md).

**Blocked by:** None (can start immediately)

**Status:** ready-for-agent

- [ ] Lấy được provider chạy được theo `provider_ref` khi có và khi không có provider runtime manager, có test cho cả hai chế độ
- [ ] Nếu việc này đòi đổi lifecycle của runtime manager thì dừng và ghi `needs-info` để trao đổi, không tự đổi
- [ ] Setting chọn provider chỉ nhận provider có tag `chat` và hỗ trợ tool; provider không hợp lệ bị từ chối khi lưu
- [ ] Setting hiện trong Settings hiện có và không cần migration; ghi lại nơi lưu đã chọn
- [ ] Chưa cấu hình thì trả `503 wiki_chat_not_configured`, không tự chọn model
- [ ] Đã cấu hình thì trả `text/event-stream` với `delta` rồi `done`; keep-alive 15 giây và `Cache-Control: no-store`
- [ ] Lịch sử quá 12 tin hoặc 16 KB, và vai `system` từ web, bị từ chối với `invalid_input`
- [ ] Quá 2 lượt đồng thời trả `429 wiki_chat_busy`; không xác thực bị từ chối
- [ ] Lỗi provider trả `error` với `provider_failed`, không chứa thông điệp nội bộ hay đường dẫn vault
- [ ] Log không có câu hỏi hay nội dung câu trả lời
- [ ] Test qua HTTP thật với LLM Provider giả
