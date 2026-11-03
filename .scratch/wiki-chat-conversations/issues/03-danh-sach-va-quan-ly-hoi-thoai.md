# 03: Danh sách và quản lý hội thoại

**What to build:** Admin mở danh sách Hội thoại từ đầu khung chat để tìm lại và chuyển sang cuộc trao đổi cũ, tạo hội thoại mới mà không mất cái cũ, đổi tên, và xóa sau khi xác nhận. Khi đang có lượt chạy thì không chuyển được hội thoại hay tạo mới, để không hủy một câu trả lời đang tốn tiền mà Admin không hay biết.

Xem spec: [Conversation routes](../spec.md#conversation-routes), [UI](../spec.md#ui).

**Blocked by:** 02

**Status:** resolved

- [x] `GET /knowledge/conversations` trả các hội thoại mới nhất trước, gồm `id`, `title`, `updated_at_ms`, `message_count`, `running`, không có nội dung tin nhắn
- [x] `PATCH` đổi được tiêu đề; tiêu đề ngoài 1–120 ký tự trả `400 invalid_input`
- [x] `DELETE` trả `204` và xóa luôn tin nhắn; `PATCH` và `DELETE` với id không tồn tại trả `404 wiki_conversation_not_found`
- [x] Nút xóa hội thoại hiện tại được thay bằng "Hội thoại mới", và hội thoại cũ vẫn còn trong danh sách
- [x] Chuyển hội thoại cập nhật URL và hiển thị đúng nội dung; xóa hội thoại đang mở đưa khung chat về trạng thái trống
- [x] Xóa phải qua bước xác nhận
- [x] Khi đang có lượt chạy, chuyển hội thoại và "Hội thoại mới" bị khóa
- [x] Ngăn kéo dưới 950px dùng được cùng danh sách; text mới có đủ cả hai ngôn ngữ giao diện
- [x] Mọi route mới từ chối yêu cầu chưa xác thực và dùng `Cache-Control: no-store`


## Implementation result

Implemented and integrated on `feat/wiki-chat-conversations`. Focused server/web validation for this ticket passed through `scripts/agent-test.sh`; see the integration commits and test record in the implementation session.
