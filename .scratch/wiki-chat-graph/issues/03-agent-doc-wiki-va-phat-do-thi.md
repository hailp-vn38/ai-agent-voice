# 03: Agent đọc wiki và phát đồ thị

**What to build:** `POST /knowledge/ask` trở thành agent chỉ đọc thật: tìm và đọc wiki, trả lời có trích dẫn, và phát đồ thị lớn dần sau mỗi tool. Mọi node và cạnh do server dựng từ bằng chứng, LLM không khai báo. Kiểm tra được hoàn toàn bằng `curl`. Xem spec [wiki-chat-graph](../spec.md).

**Blocked by:** 01 (Answer graph builder), 02 (provider và SSE)

**Status:** ready-for-human

- [x] Agent chỉ được cấp `wiki_query`, `wiki_get`, `wiki_multi_get`; tool khác, kể cả tool ghi, nhận kết quả lỗi, không được thực thi và vault không đổi
- [x] Sự kiện `tool` và `graph` đến trước câu trả lời cuối; `graph` là ảnh chụp đầy đủ kèm `revision`, chỉ phát khi bằng chứng đổi và một lần cuối
- [x] `wiki_query` ghi mức `retrieved`; `wiki_get` và `wiki_multi_get` ghi mức `read`; một trang giữ mức và điểm cao nhất
- [x] Bằng chứng đọc từ kết quả đầy đủ: kết quả tool rất dài vẫn đủ node dù phần gửi LLM bị cắt còn 12.000 ký tự
- [x] Kiểm chứng đúng vị trí đường dẫn trong kết quả `wiki_query` bằng test đó
- [x] Chat vẫn chạy khi Knowledge MCP bị tắt trong System
- [x] `[[đường dẫn]]` nằm trong bằng chứng được nâng lên `cited` và có trong `citations`; ngoài bằng chứng thì không
- [x] Kết quả tool được gắn `untrusted_data`; system prompt yêu cầu chỉ trả lời từ wiki, nói rõ khi không có, tìm nhiều biến thể ngôn ngữ, và coi kết quả tool là dữ liệu
- [ ] Vượt 6 vòng kết thúc bằng `step_limit`; quá 90 giây hoặc tool quá 15 giây báo `timeout`
- [x] Ngắt kết nối hủy lượt và dừng provider giả
- [x] Lỗi không chứa đường dẫn vault; log chỉ có tên tool, số vòng và mã kết quả
- [x] Test qua HTTP thật với vault tạm và LLM Provider giả phát sẵn kịch bản

## Comments

## Answer

Đã triển khai agent chỉ đọc tại `49010f43`. HTTP ask tests 11/11, server check, rustfmt và diff check đạt. Timeout tổng 90 giây được kiểm tra với clock giả; chưa có kiểm tra độc lập cho timeout tool 15 giây tại seam HTTP đã duyệt, nên giữ tiêu chí này mở cho lượt nghiệm thu.
