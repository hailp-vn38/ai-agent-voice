# 06: Khôi phục file ghim và xử lý trang đã mất

**What to build:** Mở lại một Wiki Conversation thì có lại file ghim của câu hỏi mới nhất, để Admin hỏi tiếp mà không phải đính kèm lại. Wiki có thể đã đổi từ lúc trả lời: câu trả lời cũ vẫn đọc được, và trích dẫn hoặc node trỏ tới trang đã xóa thì báo trang không còn thay vì lỗi hay điều hướng hỏng.

Xem spec: [Dữ liệu](../spec.md#dữ-liệu), [Answer Graph đã lưu](../spec.md#answer-graph-đã-lưu).

**Blocked by:** 02

**Status:** resolved

- [x] Mở hội thoại khôi phục file ghim từ câu hỏi mới nhất, và câu hỏi tiếp theo gửi đúng các file đó
- [x] Mỗi câu hỏi cũ vẫn hiện đúng các file đã ghim lúc gửi
- [x] Tiêu đề file được tra lúc đọc; trang đã mất thì hiện đường dẫn
- [x] Ghim file rồi tải lại mà chưa gửi thì file đó không còn (hành vi đã chốt)
- [x] Bấm trích dẫn hoặc node trỏ tới trang đã xóa thì hiện thông báo trang không còn tồn tại, và câu trả lời vẫn đọc được
- [x] Answer Graph đã lưu hiển thị đúng như lúc trả lời, không dựng lại từ wiki hiện tại


## Implementation result

Implemented and integrated on `feat/wiki-chat-conversations`. Focused server/web validation for this ticket passed through `scripts/agent-test.sh`; see the integration commits and test record in the implementation session.
