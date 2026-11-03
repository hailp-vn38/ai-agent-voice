# 04: Sự cố và chạy đồng thời

**What to build:** Một Wiki Conversation luôn ở trạng thái đúng sự thật. Lượt dở khi server dừng hiện là `interrupted` chứ không treo mãi ở `running`. Hai tab không thể chạy hai lượt cùng hội thoại. Admin thử lại được câu trả lời mới nhất bị lỗi hoặc gián đoạn, và được báo khi câu trả lời không lưu được.

Xem spec: [Vòng đời một lượt](../spec.md#vòng-đời-một-lượt), [Hợp đồng ask](../spec.md#hợp-đồng-post-knowledgeask), [UI](../spec.md#ui).

**Blocked by:** 02

**Status:** resolved

- [x] Khởi động server đổi mọi tin `running` thành `interrupted` trước khi nhận HTTP
- [x] Tin `running` quá 120 giây không chặn lượt mới của hội thoại đó và được đổi thành `interrupted`
- [x] Hai lượt cùng hội thoại: lượt sau nhận `409 wiki_conversation_busy`; hai hội thoại khác nhau vẫn chạy song song, trong giới hạn hai lượt toàn server
- [x] `DELETE` hội thoại đang chạy trả `409 wiki_conversation_busy`
- [x] Retry tin `assistant` mới nhất ở trạng thái `failed` hoặc `interrupted` ghi đè đúng dòng đó, dùng lại file ghim của câu hỏi liền trước, và lịch sử chỉ gồm các tin trước nó
- [x] Retry tin cũ hoặc tin ở trạng thái khác trả `400 invalid_input`; web chỉ hiện nút thử lại ở tin mới nhất
- [x] Ghi kết quả cuối thất bại sau 3 lần thử thì `done` có `saved: false`, và web hiện cảnh báo chưa lưu được
- [x] Web hiện `wiki_conversation_busy` kèm nút thử lại và không tự gửi lại
- [x] Mở hội thoại có tin `running` từ tab khác thì hiện chế độ chỉ đọc kèm thông báo và nút tải lại, không polling và không stream lại


## Implementation result

Implemented and integrated on `feat/wiki-chat-conversations`. Focused server/web validation for this ticket passed through `scripts/agent-test.sh`; see the integration commits and test record in the implementation session.
