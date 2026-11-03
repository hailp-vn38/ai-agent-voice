# 02: Hội thoại còn nguyên sau khi tải lại trang

**What to build:** Đường đi xuyên suốt đầu tiên của Wiki Conversation. Admin hỏi, tải lại trang và thấy lại câu trả lời, trích dẫn, bước tool và Answer Graph, rồi hỏi tiếp với đúng ngữ cảnh. Server tự ghi lượt và tự đọc lịch sử từ DB; trình duyệt không gửi lịch sử nữa. Hai bảng mới bị loại khỏi snapshot `ConfigOnly` ngay trong ticket này, để không có giai đoạn nào backup chỉ cấu hình chứa câu hỏi.

Xem spec: [Dữ liệu](../spec.md#dữ-liệu), [Hợp đồng ask](../spec.md#hợp-đồng-post-knowledgeask), [Vòng đời một lượt](../spec.md#vòng-đời-một-lượt), [Lịch sử gửi cho LLM](../spec.md#lịch-sử-gửi-cho-llm), [Backup](../spec.md#backup).

**Blocked by:** 01

**Status:** resolved

- [x] Hỏi không có `conversation_id` tạo Wiki Conversation với tiêu đề là 80 ký tự đầu của câu hỏi; hỏi có `conversation_id` thì nối tiếp hội thoại đó
- [x] `turn` là sự kiện SSE đầu tiên; sau `done`, `GET /knowledge/conversations/{id}` trả đúng câu trả lời, trích dẫn, bước tool và đồ thị
- [x] Body có `messages` bị từ chối; không có đúng một trong `question` và `retry_message_id` thì trả `400 invalid_input` (nhánh retry trả mã này cho đến ticket 04)
- [x] `conversation_id` không tồn tại trả `404 wiki_conversation_not_found`; không có DB trả `503 database_unavailable`
- [x] Lịch sử LLM nhận được đến từ DB, đúng giới hạn 12 tin / 16 KiB, không gồm tin `assistant` chưa `done`, và không bắt đầu bằng tin `assistant`
- [x] Input sai, thiếu cấu hình provider và hết permit không tạo dòng nào
- [x] Bấm Dừng hoặc ngắt kết nối để lại tin `stopped` kèm phần đã stream; lỗi provider, `timeout`, `step_limit` để lại tin `failed` với đúng mã
- [x] Tải trang với `?conversation=<id>` hiển thị lại tin nhắn, trích dẫn và Answer Graph của câu trả lời mới nhất có đồ thị
- [x] Gửi câu đầu cập nhật URL theo sự kiện `turn`; gửi tiếp dùng đúng `conversation_id`; id không tồn tại trong URL thì bỏ tham số và mở trạng thái trống
- [x] Snapshot `ConfigOnly` không còn dòng nào trong hai bảng; snapshot `Full` còn đủ
- [x] Lỗi và log không chứa câu hỏi, nội dung trang hay đường dẫn tuyệt đối; route mới từ chối yêu cầu chưa xác thực


## Implementation result

Implemented and integrated on `feat/wiki-chat-conversations`. Focused server/web validation for this ticket passed through `scripts/agent-test.sh`; see the integration commits and test record in the implementation session.
